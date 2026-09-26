//! Core Local Interruptor (CLINT).
//!
//! Holds the machine software-interrupt and timer registers of every hart
//! and the single `mtime` counter they compare against, at the standard
//! SiFive/QEMU `virt` layout.
//!
//! # Memory Map
//!
//! * `0x0000 + 4·hart`: MSIP (Machine Software Interrupt Pending)
//! * `0x4000 + 8·hart`: MTIMECMP (Machine Time Compare)
//! * `0xBFF8`: MTIME (Machine Time)

use crate::common::{HartId, LineAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;

/// Offset of the first MSIP register; one 4-byte word per hart.
const MSIP_BASE: u64 = 0x0000;
/// Offset of the first MTIMECMP register; one 8-byte word per hart.
const MTIMECMP_BASE: u64 = 0x4000;
/// Offset of the MTIME register.
const MTIME_OFFSET: u64 = 0xBFF8;
/// Size of the device's MMIO window.
const CLINT_SIZE: u64 = 0x10000;

const MSIP_STRIDE: u64 = 4;
const MTIMECMP_STRIDE: u64 = 8;

/// CLINT device structure.
#[derive(Debug)]
pub struct Clint {
    /// Base physical address of the device.
    base_addr: u64,
    /// Machine time counter shared by every hart.
    mtime: u64,
    /// Per-hart machine time compare registers.
    mtimecmp: Vec<u64>,
    /// Per-hart machine software interrupt pending bits.
    msip: Vec<u32>,
    /// Divider to scale CPU cycles to timer ticks.
    divider: u64,
    /// Internal counter for the divider.
    counter: u64,
}

impl Clint {
    /// Creates a CLINT serving `hart_count` harts.
    ///
    /// `divider` is the ratio of CPU cycles to timer ticks (10 means `mtime`
    /// increments every 10 cycles); zero is treated as one.
    pub fn new(base_addr: u64, divider: u64, hart_count: usize) -> Self {
        Self {
            base_addr,
            mtime: 0,
            mtimecmp: vec![u64::MAX; hart_count],
            msip: vec![0; hart_count],
            divider: if divider == 0 { 1 } else { divider },
            counter: 0,
        }
    }

    /// Number of harts this CLINT serves.
    #[must_use]
    pub const fn hart_count(&self) -> usize {
        self.msip.len()
    }

    /// Current value of the shared `mtime` counter.
    #[must_use]
    pub const fn mtime(&self) -> u64 {
        self.mtime
    }

    /// Returns `true` when `hart`'s machine software interrupt is pending.
    #[must_use]
    pub fn msip_pending(&self, hart: HartId) -> bool {
        self.msip.get(hart.as_index()).is_some_and(|msip| (msip & 1) != 0)
    }

    /// Returns `true` when `mtime` has reached `hart`'s `mtimecmp`.
    #[must_use]
    pub fn timer_pending(&self, hart: HartId) -> bool {
        self.mtimecmp.get(hart.as_index()).is_some_and(|cmp| self.mtime >= *cmp)
    }

    /// Reads the 8-byte-aligned window containing `offset`.
    fn read_window(&self, offset: u64) -> u64 {
        let aligned = offset & !7;
        if aligned < MTIMECMP_BASE {
            let first = (aligned - MSIP_BASE) / MSIP_STRIDE;
            let lo = self.msip.get(first as usize).copied().unwrap_or(0);
            let hi = self.msip.get(first as usize + 1).copied().unwrap_or(0);
            return u64::from(lo) | (u64::from(hi) << 32);
        }
        if aligned == MTIME_OFFSET {
            return self.mtime;
        }
        if aligned >= MTIMECMP_BASE {
            let hart = (aligned - MTIMECMP_BASE) / MTIMECMP_STRIDE;
            return self.mtimecmp.get(hart as usize).copied().unwrap_or(0);
        }
        0
    }

    fn read_register(&self, offset: u64, size: AccessSize) -> u64 {
        let window = self.read_window(offset);
        let shift = (offset & 7) * 8;
        match size {
            AccessSize::B8 | AccessSize::Line => window,
            AccessSize::B4 => (window >> shift) & 0xFFFF_FFFF,
            AccessSize::B2 => (window >> shift) & 0xFFFF,
            AccessSize::B1 => (window >> shift) & 0xFF,
        }
    }

    /// Writes one 4-byte-aligned register word.
    fn write_word(&mut self, offset: u64, val: u32) {
        let aligned = offset & !3;
        if aligned < MTIMECMP_BASE {
            let hart = ((aligned - MSIP_BASE) / MSIP_STRIDE) as usize;
            if let Some(msip) = self.msip.get_mut(hart) {
                *msip = val & 1;
            }
            return;
        }
        if aligned == MTIME_OFFSET {
            self.mtime = (self.mtime & 0xFFFF_FFFF_0000_0000) | u64::from(val);
            return;
        }
        if aligned == MTIME_OFFSET + 4 {
            self.mtime = (self.mtime & 0x0000_0000_FFFF_FFFF) | (u64::from(val) << 32);
            return;
        }
        if aligned >= MTIMECMP_BASE {
            let hart = ((aligned - MTIMECMP_BASE) / MTIMECMP_STRIDE) as usize;
            let upper_half = (aligned & 4) != 0;
            if let Some(cmp) = self.mtimecmp.get_mut(hart) {
                *cmp = if upper_half {
                    (*cmp & 0x0000_0000_FFFF_FFFF) | (u64::from(val) << 32)
                } else {
                    (*cmp & 0xFFFF_FFFF_0000_0000) | u64::from(val)
                };
            }
        }
    }

    fn write_register(&mut self, offset: u64, size: AccessSize, val: u64) {
        match size {
            AccessSize::B8 => {
                let aligned = offset & !7;
                self.write_word(aligned, val as u32);
                self.write_word(aligned + 4, (val >> 32) as u32);
            }
            AccessSize::B4 => self.write_word(offset, val as u32),
            AccessSize::B1 | AccessSize::B2 | AccessSize::Line => {}
        }
    }
}

impl Handle for Clint {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let value = match op {
                MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                    self.read_register(offset, size)
                }
                MemOp::Write { data: WriteData::Small(val) } => {
                    self.write_register(offset, size, val);
                    0
                }
                MemOp::Write { .. } | MemOp::Writeback { .. } => 0,
            };
            ctx.scheduler.schedule(
                ctx.cycle + 1,
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, 64),
                    data: MemRespData::Small(value),
                    hit_level: HitLevel::Mmio,
                    state: MesiState::Exclusive,
                },
            );
        }
    }
}

impl Device for Clint {
    fn name(&self) -> &'static str {
        "CLINT"
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, CLINT_SIZE)
    }

    /// Advances `mtime` by one divider step. Timer and software interrupt
    /// lines are read per hart through [`Clint::timer_pending`] and
    /// [`Clint::msip_pending`]; the CLINT raises no PLIC source.
    fn tick(&mut self) -> bool {
        self.counter += 1;
        if self.counter >= self.divider {
            self.mtime = self.mtime.wrapping_add(1);
            self.counter = 0;
        }
        false
    }

    fn as_clint(&self) -> Option<&Clint> {
        Some(self)
    }

    fn as_clint_mut(&mut self) -> Option<&mut Clint> {
        Some(self)
    }
}
