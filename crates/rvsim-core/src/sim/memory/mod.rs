//! The one image of memory every access takes effect against.
//!
//! Caches hold tags only, so every line's data lives here, with the LR/SC
//! reservations and the log of writes that make other harts' writes
//! observable.

mod ram;
pub mod reservations;
pub mod write_log;

pub use ram::Ram;

#[cfg(feature = "commit-log")]
use crate::common::usize_of;
use crate::common::{PhysAddr, PhysRange};
use crate::exec::compute::amo;
use crate::isa::op::{AtomicOp, MemWidth};
use crate::sim::packet::{AccessSize, MemOp, MemRespData, WriteData, WriteOrigin};
use reservations::ReservationSet;
use write_log::{WriteLog, Writer};

/// RAM something other than a hart wrote: a device's DMA, HTIF or the
/// SE-mode host. A commit log records it, since no instruction did.
#[cfg(feature = "commit-log")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostWrite {
    /// Where the write began.
    pub paddr: PhysAddr,
    /// What it wrote, in address order.
    pub bytes: Vec<u8>,
}

/// RAM with the reservations and write log that go with it.
#[derive(Debug)]
pub struct GlobalMemory {
    ram: Option<Ram>,
    reservations: ReservationSet,
    write_log: Option<WriteLog>,
    #[cfg(feature = "commit-log")]
    host_writes: Option<Vec<HostWrite>>,
}

impl GlobalMemory {
    /// Memory over `ram` shared by `hart_count` harts; the write log is kept
    /// only when more than one hart can write, at `line_bytes` granularity.
    #[must_use]
    pub fn new(ram: Option<Ram>, hart_count: usize, line_bytes: u64) -> Self {
        let write_log = ram
            .as_ref()
            .filter(|_| hart_count > 1)
            .map(|ram| WriteLog::new(ram.base().val(), ram.size(), line_bytes, hart_count));
        Self {
            ram,
            reservations: ReservationSet::new(hart_count),
            write_log,
            #[cfg(feature = "commit-log")]
            host_writes: None,
        }
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

    /// Keeps every write from outside the harts from now on, for
    /// [`Self::take_host_writes`].
    #[cfg(feature = "commit-log")]
    pub fn record_host_writes(&mut self) {
        let _ = self.host_writes.get_or_insert_with(Vec::new);
    }

    /// The writes from outside the harts since the last call; none unless
    /// [`Self::record_host_writes`] was called.
    #[cfg(feature = "commit-log")]
    pub fn take_host_writes(&mut self) -> Vec<HostWrite> {
        self.host_writes.as_mut().map(std::mem::take).unwrap_or_default()
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

    /// The `N` bytes at `paddr` in address order; `None` outside RAM.
    #[must_use]
    pub fn read_array<const N: usize>(&self, paddr: PhysAddr) -> Option<[u8; N]> {
        self.ram.as_ref()?.get(paddr, N)?.try_into().ok()
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
        self.note_write(writer, PhysRange::new(paddr, bytes.len() as u64));
    }

    /// Writes `bytes` at `paddr` as `writer`, noting the write on every
    /// line it touches. Outside RAM nothing is written.
    pub fn write_bytes(&mut self, writer: Writer, paddr: PhysAddr, bytes: &[u8]) {
        let Some(slice) = self.ram.as_mut().and_then(|ram| ram.get_mut(paddr, bytes.len())) else {
            return;
        };
        slice.copy_from_slice(bytes);
        self.note_write(writer, PhysRange::new(paddr, bytes.len() as u64));
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
        self.note_write(writer, PhysRange::new(line, bytes.len() as u64));
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
            MemOp::Atomic { op: AtomicOp::Amo(op), data, hart } => {
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
            | MemOp::Maintain { .. }
            | MemOp::Prefetch { .. } => MemRespData::Small(0),
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

    /// Breaks the reservations and logs the write `writer` made of the bytes
    /// `written`.
    fn note_write(&mut self, writer: Writer, written: PhysRange) {
        #[cfg(feature = "commit-log")]
        if writer == Writer::External
            && let Some(journal) = self.host_writes.as_mut()
            && let Some(bytes) =
                self.ram.as_ref().and_then(|ram| ram.get(written.base, usize_of(written.size)))
        {
            journal.push(HostWrite { paddr: written.base, bytes: bytes.to_vec() });
        }
        self.reservations.break_on_write(writer, written);
        if let Some(log) = self.write_log.as_mut() {
            log.record(written, writer);
        }
    }
}
