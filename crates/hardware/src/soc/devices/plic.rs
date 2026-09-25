//! Platform-Level Interrupt Controller (PLIC).
//!
//! The PLIC arbitrates global external interrupts and distributes them to
//! interrupt targets. Every hart has two contexts in the standard layout:
//! context `2·hart` is its M-mode target and `2·hart + 1` its S-mode target.
//!
//! # Memory Map
//!
//! * `0x000000`: Interrupt Priorities
//! * `0x001000`: Interrupt Pending Bits
//! * `0x002000 + 0x80·context`: Interrupt Enables
//! * `0x200000 + 0x1000·context`: Priority Threshold and Claim/Complete

use crate::common::{HartId, LineAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, Packet, WriteData};
use crate::soc::devices::Device;

/// Base offset for PLIC priority registers (one per interrupt source).
const PLIC_PRIORITY_BASE: u64 = 0x000000;

/// Base offset for PLIC pending interrupt register.
const PLIC_PENDING_BASE: u64 = 0x001000;

/// Base offset for PLIC interrupt enable registers (per context).
const PLIC_ENABLE_BASE: u64 = 0x002000;

/// Base offset for PLIC context-specific registers (threshold, claim/complete).
const PLIC_CONTEXT_BASE: u64 = 0x200000;

/// Byte stride between one context's enable block and the next.
const ENABLE_STRIDE: u64 = 0x80;

/// Byte stride between one context's threshold/claim block and the next.
const CONTEXT_STRIDE: u64 = 0x1000;

/// Contexts per hart: M-mode and S-mode.
const CONTEXTS_PER_HART: usize = 2;

/// Number of 32-bit enable words per context (covers 1024 interrupt sources).
const ENABLE_WORDS_PER_CONTEXT: usize = 32;

/// Number of interrupt sources the PLIC decodes priorities for.
const NUM_SOURCES: usize = 1024;

/// External interrupt lines the PLIC drives into one hart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExternalIrqs {
    /// Machine external interrupt (`mip.MEIP`).
    pub meip: bool,
    /// Supervisor external interrupt (`mip.SEIP`).
    pub seip: bool,
}

/// PLIC device structure.
#[derive(Debug)]
pub struct Plic {
    /// Base physical address of the device.
    base_addr: u64,
    /// Interrupt source priorities (1-1023).
    priorities: Vec<u32>,
    /// Pending interrupt bits (bitmap).
    pending: Vec<u32>,
    /// Interrupt enable bits per context: enables[ctx][word].
    enables: Vec<Vec<u32>>,
    /// Priority thresholds per context.
    thresholds: Vec<u32>,
    /// Claim/Complete registers per context.
    claims: Vec<u32>,
}

impl Plic {
    /// Creates a PLIC with two contexts for each of `hart_count` harts.
    pub fn new(base_addr: u64, hart_count: usize) -> Self {
        let contexts = hart_count * CONTEXTS_PER_HART;
        Self {
            base_addr,
            priorities: vec![0; NUM_SOURCES],
            pending: vec![0; NUM_SOURCES / 32],
            enables: vec![vec![0u32; ENABLE_WORDS_PER_CONTEXT]; contexts],
            thresholds: vec![0; contexts],
            claims: vec![0; contexts],
        }
    }

    /// Number of interrupt contexts (two per hart).
    #[must_use]
    pub const fn context_count(&self) -> usize {
        self.thresholds.len()
    }

    /// Updates the pending status of interrupts based on external signals.
    pub fn update_irqs(&mut self, mask: u64) {
        self.pending[0] = (mask & 0xFFFF_FFFF) as u32;
        self.pending[1] = (mask >> 32) as u32;
    }

    /// Recomputes every context's claim register from the pending, enable
    /// and threshold state. Call once per cycle after [`Plic::update_irqs`].
    pub fn check_interrupts(&mut self) {
        for ctx in 0..self.context_count() {
            self.claims[ctx] = self.calc_max_id(ctx);
        }
    }

    /// External interrupt lines for `hart` as of the last
    /// [`Plic::check_interrupts`].
    #[must_use]
    pub fn hart_lines(&self, hart: HartId) -> ExternalIrqs {
        let m_ctx = hart.as_index() * CONTEXTS_PER_HART;
        ExternalIrqs {
            meip: self.claims.get(m_ctx).is_some_and(|c| *c != 0),
            seip: self.claims.get(m_ctx + 1).is_some_and(|c| *c != 0),
        }
    }

    /// Calculates the ID of the highest-priority pending, enabled interrupt
    /// above the context's threshold, or 0 when there is none.
    fn calc_max_id(&self, ctx: usize) -> u32 {
        let threshold = self.thresholds[ctx];
        let num_words = std::cmp::min(self.pending.len(), self.enables[ctx].len());

        let mut max_prio = 0;
        let mut max_id = 0;

        for word in 0..num_words {
            let active = self.pending[word] & self.enables[ctx][word];
            if active == 0 {
                continue;
            }
            for bit in 0..32 {
                let irq_id = word * 32 + bit;
                if irq_id == 0 {
                    continue;
                }
                if (active & (1 << bit)) != 0 && irq_id < self.priorities.len() {
                    let prio = self.priorities[irq_id];
                    if prio > max_prio && prio > threshold {
                        max_prio = prio;
                        max_id = irq_id as u32;
                    }
                }
            }
        }
        max_id
    }

    fn clear_pending(&mut self, irq_id: u32) {
        if irq_id > 0 && (irq_id as usize) < NUM_SOURCES {
            let idx = irq_id as usize / 32;
            let bit = 1u32 << (irq_id % 32);
            if idx < self.pending.len() {
                self.pending[idx] &= !bit;
            }
        }
    }
}

impl Plic {
    fn read_u32_reg(&mut self, offset: u64) -> u32 {
        #[allow(clippy::absurd_extreme_comparisons)]
        if (PLIC_PRIORITY_BASE..PLIC_PENDING_BASE).contains(&offset) {
            let idx = (offset - PLIC_PRIORITY_BASE) as usize / 4;
            if idx < self.priorities.len() {
                return self.priorities[idx];
            }
        } else if (PLIC_PENDING_BASE..PLIC_ENABLE_BASE).contains(&offset) {
            let idx = (offset - PLIC_PENDING_BASE) as usize / 4;
            if idx < self.pending.len() {
                return self.pending[idx];
            }
        } else if (PLIC_ENABLE_BASE..PLIC_CONTEXT_BASE).contains(&offset) {
            let rel = offset - PLIC_ENABLE_BASE;
            let ctx = (rel / ENABLE_STRIDE) as usize;
            let word_idx = ((rel % ENABLE_STRIDE) / 4) as usize;
            if ctx < self.context_count() && word_idx < ENABLE_WORDS_PER_CONTEXT {
                return self.enables[ctx][word_idx];
            }
        } else if offset >= PLIC_CONTEXT_BASE {
            let ctx = ((offset - PLIC_CONTEXT_BASE) / CONTEXT_STRIDE) as usize;
            let reg = offset & 0xFFF;
            if ctx < self.context_count() {
                if reg == 0 {
                    return self.thresholds[ctx];
                }
                if reg == 4 {
                    let irq_id = self.claims[ctx];
                    self.clear_pending(irq_id);
                    return irq_id;
                }
            }
        }
        0
    }

    fn write_u32_reg(&mut self, offset: u64, val: u32) {
        #[allow(clippy::absurd_extreme_comparisons)]
        if (PLIC_PRIORITY_BASE..PLIC_PENDING_BASE).contains(&offset) {
            let idx = (offset - PLIC_PRIORITY_BASE) as usize / 4;
            if idx < self.priorities.len() {
                self.priorities[idx] = val;
            }
        } else if (PLIC_ENABLE_BASE..PLIC_CONTEXT_BASE).contains(&offset) {
            let rel = offset - PLIC_ENABLE_BASE;
            let ctx = (rel / ENABLE_STRIDE) as usize;
            let word_idx = ((rel % ENABLE_STRIDE) / 4) as usize;
            if ctx < self.context_count() && word_idx < ENABLE_WORDS_PER_CONTEXT {
                self.enables[ctx][word_idx] = val;
            }
        } else if offset >= PLIC_CONTEXT_BASE {
            let ctx = ((offset - PLIC_CONTEXT_BASE) / CONTEXT_STRIDE) as usize;
            let reg = offset & 0xFFF;
            if ctx < self.context_count() {
                if reg == 0 {
                    self.thresholds[ctx] = val;
                }
                if reg == 4 {
                    self.clear_pending(val);
                    self.claims[ctx] = 0;
                }
            }
        }
    }
}

impl Handle for Plic {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let value: u64 = match (size, op) {
                (
                    AccessSize::B4 | AccessSize::B8,
                    MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. },
                ) => u64::from(self.read_u32_reg(offset)),
                (
                    AccessSize::B1,
                    MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. },
                ) => {
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    u64::from((self.read_u32_reg(aligned) >> shift) as u8)
                }
                (AccessSize::B2, MemOp::Read | MemOp::Fetch | MemOp::Atomic { .. }) => {
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    u64::from((self.read_u32_reg(aligned) >> shift) as u16)
                }
                (
                    AccessSize::B4 | AccessSize::B8,
                    MemOp::Write { data: WriteData::Small(val) },
                ) => {
                    self.write_u32_reg(offset, val as u32);
                    0
                }
                _ => 0,
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
                },
            );
        }
    }
}

impl Device for Plic {
    fn name(&self) -> &'static str {
        "PLIC"
    }
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x4000000)
    }

    fn as_plic_mut(&mut self) -> Option<&mut Plic> {
        Some(self)
    }
}
