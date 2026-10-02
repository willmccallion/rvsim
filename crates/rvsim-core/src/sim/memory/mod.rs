//! The one image of memory every access takes effect against.
//!
//! Caches hold tags only, so every line's data lives here, with the LR/SC
//! reservations and the log of writes that make other harts' writes
//! observable.

mod ram;
pub mod reservations;
pub mod write_log;

pub use ram::Ram;

use crate::common::PhysAddr;
use crate::exec::compute::amo;
use crate::isa::op::{AtomicOp, MemWidth};
use crate::sim::packet::{AccessSize, MemOp, MemRespData, WriteData, WriteOrigin};
use reservations::ReservationSet;
use write_log::{WriteLog, Writer};

/// RAM with the reservations and write log that go with it.
#[derive(Debug)]
pub struct GlobalMemory {
    ram: Option<Ram>,
    reservations: ReservationSet,
    write_log: Option<WriteLog>,
}

impl GlobalMemory {
    /// Memory over `ram` shared by `hart_count` harts; the write log is kept
    /// only when more than one hart can write, at `line_bytes` granularity.
    #[must_use]
    pub fn new(ram: Option<Ram>, hart_count: usize, line_bytes: u64) -> Self {
        let write_log = ram
            .as_ref()
            .filter(|_| hart_count > 1)
            .map(|ram| WriteLog::new(ram.base(), ram.size(), line_bytes, hart_count));
        Self { ram, reservations: ReservationSet::new(hart_count), write_log }
    }

    /// The RAM image; `None` in a system without RAM.
    #[must_use]
    pub const fn ram(&self) -> Option<&Ram> {
        self.ram.as_ref()
    }

    /// The RAM image, to overwrite wholesale (a checkpoint restore).
    pub const fn ram_mut(&mut self) -> Option<&mut Ram> {
        self.ram.as_mut()
    }

    /// The LR/SC reservations.
    #[must_use]
    pub const fn reservations(&self) -> &ReservationSet {
        &self.reservations
    }

    /// The LR/SC reservations, for setting and clearing.
    pub const fn reservations_mut(&mut self) -> &mut ReservationSet {
        &mut self.reservations
    }

    /// The write log; `None` with a single hart.
    #[must_use]
    pub const fn write_log(&self) -> Option<&WriteLog> {
        self.write_log.as_ref()
    }

    /// The `bytes` (at most 8) at `paddr`, little-endian; `None` outside RAM.
    #[must_use]
    pub fn read(&self, paddr: PhysAddr, bytes: usize) -> Option<u64> {
        let slice = self.ram.as_ref()?.get(paddr, bytes)?;
        let mut word = [0u8; 8];
        word.get_mut(..bytes)?.copy_from_slice(slice);
        Some(u64::from_le_bytes(word))
    }

    /// The `len` bytes at `paddr` in address order; `None` outside RAM.
    #[must_use]
    pub fn read_bytes(&self, paddr: PhysAddr, len: usize) -> Option<Box<[u8]>> {
        Some(self.ram.as_ref()?.get(paddr, len)?.into())
    }

    /// Writes the low `bytes` (at most 8) of `data` at `paddr` as `writer`,
    /// breaking the reservations the write must break. Outside RAM nothing
    /// is written.
    pub fn write(&mut self, writer: Writer, paddr: PhysAddr, data: u64, bytes: usize) {
        let word = data.to_le_bytes();
        let Some(bytes) = word.get(..bytes) else { return };
        let Some(slice) = self.ram.as_mut().and_then(|ram| ram.get_mut(paddr, bytes.len())) else {
            return;
        };
        slice.copy_from_slice(bytes);
        self.note_write(writer, paddr);
    }

    /// Writes `bytes` at `paddr` as `writer`, noting the write on every
    /// line it touches. Outside RAM nothing is written.
    pub fn write_bytes(&mut self, writer: Writer, paddr: PhysAddr, bytes: &[u8]) {
        let Some(slice) = self.ram.as_mut().and_then(|ram| ram.get_mut(paddr, bytes.len())) else {
            return;
        };
        slice.copy_from_slice(bytes);
        self.note_write_range(writer, paddr, bytes.len());
    }

    /// Places `image` at `paddr` before the system runs: no reservation
    /// breaks and no log entry, as nothing has observed memory yet. Outside
    /// RAM nothing is written.
    pub fn load(&mut self, paddr: PhysAddr, image: &[u8]) {
        if let Some(slice) = self.ram.as_mut().and_then(|ram| ram.get_mut(paddr, image.len())) {
            slice.copy_from_slice(image);
        }
    }

    /// Writes the bytes of the line at `line` that `mask` selects, as
    /// `writer`. Outside RAM nothing is written.
    fn write_line(&mut self, writer: Writer, line: PhysAddr, bytes: &[u8], mask: u64) {
        let Some(slice) = self.ram.as_mut().and_then(|ram| ram.get_mut(line, bytes.len())) else {
            return;
        };
        for (i, (dst, src)) in slice.iter_mut().zip(bytes).enumerate() {
            if mask >> i & 1 == 1 {
                *dst = *src;
            }
        }
        self.note_write(writer, line);
    }

    /// Makes a hart's access take effect on the `size` bytes at `paddr`
    /// now, where the memory system serves it (see
    /// [`MemOp::takes_effect_when_served`]), and returns what it read.
    pub fn perform(&mut self, paddr: PhysAddr, size: AccessSize, op: &MemOp) -> MemRespData {
        match op {
            MemOp::Read if matches!(size, AccessSize::Span(_)) => MemRespData::PerformedBytes {
                bytes: self
                    .read_bytes(paddr, size.bytes())
                    .unwrap_or_else(|| vec![0; size.bytes()].into()),
                observed: self.write_log.as_ref().map(WriteLog::now),
            },
            MemOp::Read | MemOp::Atomic { op: AtomicOp::Lr, .. } => {
                self.performed(self.read(paddr, size.bytes()))
            }
            MemOp::Atomic { op: AtomicOp::Sc, data, hart } => {
                let succeeds = self.reservations.check(*hart, paddr);
                if succeeds {
                    self.write(Writer::Hart(*hart), paddr, *data, size.bytes());
                }
                self.reservations.clear(*hart);
                self.performed(Some(u64::from(!succeeds)))
            }
            MemOp::Atomic { op, data, hart } => {
                let old = self.read(paddr, size.bytes());
                let width = if size.bytes() == 4 { MemWidth::Word } else { MemWidth::Double };
                let new = amo::atomic_alu(*op, old.unwrap_or(0), *data, width);
                let response = self.performed(old);
                self.write(Writer::Hart(*hart), paddr, new, size.bytes());
                response
            }
            MemOp::Write { data, origin: WriteOrigin::Hart(hart) } => {
                let writer = Writer::Hart(*hart);
                match data {
                    WriteData::Small(value) => self.write(writer, paddr, *value, size.bytes()),
                    WriteData::Line { bytes, mask } => self.write_line(writer, paddr, bytes, *mask),
                }
                MemRespData::Small(0)
            }
            MemOp::ReadOwn
            | MemOp::Write { .. }
            | MemOp::Fetch
            | MemOp::Writeback { .. }
            | MemOp::Maintain { .. } => MemRespData::Small(0),
        }
    }

    /// A response carrying `value` (zero outside RAM), stamped with the
    /// write order it reflects.
    fn performed(&self, value: Option<u64>) -> MemRespData {
        MemRespData::Performed {
            value: value.unwrap_or(0),
            observed: self.write_log.as_ref().map(WriteLog::now),
        }
    }

    /// Notes a write of `len` bytes from `paddr`, line by line.
    fn note_write_range(&mut self, writer: Writer, paddr: PhysAddr, len: usize) {
        let line_bytes = self.write_log.as_ref().map_or(64, WriteLog::line_bytes);
        let first = paddr.val() / line_bytes;
        let last = paddr.val().saturating_add(len.saturating_sub(1) as u64) / line_bytes;
        for line in first..=last {
            self.note_write(writer, PhysAddr::new(line * line_bytes));
        }
    }

    fn note_write(&mut self, writer: Writer, paddr: PhysAddr) {
        match writer {
            Writer::Hart(hart) => self.reservations.invalidate_others(hart, paddr),
            Writer::External => self.reservations.invalidate_all(paddr),
        }
        if let Some(log) = self.write_log.as_mut() {
            log.record(paddr, writer);
        }
    }
}
