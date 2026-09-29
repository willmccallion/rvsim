//! The one image of memory every access takes effect against.
//!
//! Caches hold tags only, so every line's data lives here, with the LR/SC
//! reservations and the log of writes that make other harts' writes
//! observable.

use super::reservations::ReservationSet;
use super::write_log::{WriteLog, Writer};
use crate::common::PhysAddr;
use crate::exec::compute::amo;
use crate::isa::op::{self, MemWidth};
use crate::sim::packet::{AccessSize, AtomicOp, MemOp, MemRespData, WriteData, WriteOrigin};
use crate::soc::memory::RamRegion;

/// RAM with the reservations and write log that go with it.
#[derive(Debug)]
pub struct GlobalMemory {
    ram: Option<RamRegion>,
    reservations: ReservationSet,
    write_log: Option<WriteLog>,
}

impl GlobalMemory {
    /// Memory over `ram` shared by `hart_count` harts; the write log is kept
    /// only when more than one hart can write, at `line_bytes` granularity.
    #[must_use]
    pub fn new(ram: Option<RamRegion>, hart_count: usize, line_bytes: u64) -> Self {
        let write_log = ram
            .filter(|_| hart_count > 1)
            .map(|ram| WriteLog::new(ram.base(), ram.size(), line_bytes, hart_count));
        Self { ram, reservations: ReservationSet::new(hart_count), write_log }
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
        let ram = self.ram.filter(|ram| ram.contains(paddr.val(), bytes as u64))?;
        let value = (0..bytes).fold(0u64, |value, i| {
            // SAFETY: `contains` bounds-checked `[paddr, paddr + bytes)`.
            let byte = unsafe { *ram.ptr(paddr.val() + i as u64) };
            value | (u64::from(byte) << (8 * i))
        });
        Some(value)
    }

    /// The `len` bytes at `paddr` in address order; `None` outside RAM.
    #[must_use]
    pub fn read_bytes(&self, paddr: PhysAddr, len: usize) -> Option<Box<[u8]>> {
        let ram = self.ram.filter(|ram| ram.contains(paddr.val(), len as u64))?;
        let bytes = (0..len)
            // SAFETY: `contains` bounds-checked `[paddr, paddr + len)`.
            .map(|i| unsafe { *ram.ptr(paddr.val() + i as u64) })
            .collect();
        Some(bytes)
    }

    /// Writes the low `bytes` (at most 8) of `data` at `paddr` as `writer`,
    /// breaking the reservations the write must break. Outside RAM nothing
    /// is written.
    pub fn write(&mut self, writer: Writer, paddr: PhysAddr, data: u64, bytes: usize) {
        let Some(ram) = self.ram.filter(|ram| ram.contains(paddr.val(), bytes as u64)) else {
            return;
        };
        for i in 0..bytes {
            // SAFETY: `contains` bounds-checked `[paddr, paddr + bytes)`.
            unsafe { *ram.ptr(paddr.val() + i as u64) = (data >> (8 * i)) as u8 };
        }
        self.note_write(writer, paddr);
    }

    /// Writes the bytes of the line at `line` that `mask` selects, as
    /// `writer`. Outside RAM nothing is written.
    fn write_line(&mut self, writer: Writer, line: PhysAddr, bytes: &[u8], mask: u64) {
        let Some(ram) = self.ram.filter(|ram| ram.contains(line.val(), bytes.len() as u64)) else {
            return;
        };
        for (i, byte) in bytes.iter().enumerate().filter(|&(i, _)| mask >> i & 1 == 1) {
            // SAFETY: `contains` bounds-checked the whole line.
            unsafe { *ram.ptr(line.val() + i as u64) = *byte };
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
                let new = amo::atomic_alu(alu_op(*op), old.unwrap_or(0), *data, width);
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

    /// Records a write that bypassed [`Self::write`] (the loader, a
    /// host-side probe, or a device's DMA writing RAM directly).
    pub fn record_external_write(&mut self, paddr: PhysAddr) {
        self.note_write(Writer::External, paddr);
    }

    /// Records an external write of `len` bytes from `paddr`, line by line.
    pub fn record_external_write_range(&mut self, paddr: PhysAddr, len: usize) {
        let line_bytes = self.write_log.as_ref().map_or(64, WriteLog::line_bytes);
        let first = paddr.val() / line_bytes;
        let last = paddr.val().saturating_add(len.saturating_sub(1) as u64) / line_bytes;
        for line in first..=last {
            self.note_write(Writer::External, PhysAddr::new(line * line_bytes));
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

/// The pipeline's name for an AMO's operation.
const fn alu_op(op: AtomicOp) -> op::AtomicOp {
    match op {
        AtomicOp::Add => op::AtomicOp::Add,
        AtomicOp::Swap => op::AtomicOp::Swap,
        AtomicOp::Xor => op::AtomicOp::Xor,
        AtomicOp::And => op::AtomicOp::And,
        AtomicOp::Or => op::AtomicOp::Or,
        AtomicOp::Min => op::AtomicOp::Min,
        AtomicOp::Max => op::AtomicOp::Max,
        AtomicOp::MinU => op::AtomicOp::Minu,
        AtomicOp::MaxU => op::AtomicOp::Maxu,
        AtomicOp::Lr => op::AtomicOp::Lr,
        AtomicOp::Sc => op::AtomicOp::Sc,
    }
}
