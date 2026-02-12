//! Platform-Level Interrupt Controller (PLIC).
//!
//! The PLIC arbitrates global external interrupts and distributes them to
//! interrupt targets (HART contexts). It complies with the RISC-V PLIC specification.
//!
//! # Memory Map
//!
//! * `0x000000`: Interrupt Priorities
//! * `0x001000`: Interrupt Pending Bits
//! * `0x002000`: Interrupt Enables
//! * `0x200000`: Priority Thresholds and Claim/Complete Registers

use crate::soc::devices::Device;

/// Base offset for PLIC priority registers (one per interrupt source).
const PLIC_PRIORITY_BASE: u64 = 0x000000;

/// Base offset for PLIC pending interrupt register.
const PLIC_PENDING_BASE: u64 = 0x001000;

/// Base offset for PLIC interrupt enable registers (per context).
const PLIC_ENABLE_BASE: u64 = 0x002000;

/// Base offset for PLIC context-specific registers (threshold, claim/complete).
const PLIC_CONTEXT_BASE: u64 = 0x200000;

/// Number of interrupt contexts (M-mode + S-mode per HART).
const NUM_CONTEXTS: usize = 2;

/// Number of 32-bit enable words per context (covers 1024 interrupt sources).
const ENABLE_WORDS_PER_CONTEXT: usize = 32;

/// PLIC device structure.
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
    /// Creates a new PLIC device.
    pub fn new(base_addr: u64) -> Self {
        Self {
            base_addr,
            priorities: vec![0; 1024],
            pending: vec![0; 32],
            enables: vec![vec![0u32; ENABLE_WORDS_PER_CONTEXT]; NUM_CONTEXTS],
            thresholds: vec![0; NUM_CONTEXTS],
            claims: vec![0; NUM_CONTEXTS],
        }
    }

    /// Updates the pending status of interrupts based on external signals.
    ///
    /// # Arguments
    ///
    /// * `mask` - A 64-bit mask where set bits indicate active interrupt lines.
    pub fn update_irqs(&mut self, mask: u64) {
        self.pending[0] = (mask & 0xFFFFFFFF) as u32;
        self.pending[1] = (mask >> 32) as u32;
    }

    /// Checks for pending interrupts that exceed the priority threshold.
    ///
    /// # Returns
    ///
    /// A tuple `(meip, seip)` indicating if a Machine External Interrupt
    /// or Supervisor External Interrupt is pending.
    pub fn check_interrupts(&mut self) -> (bool, bool) {
        let mut meip = false;
        let mut seip = false;

        if self.has_qualified_irq(0) {
            meip = true;
            self.claims[0] = self.calc_max_id(0);
        } else {
            self.claims[0] = 0;
        }

        if self.has_qualified_irq(1) {
            seip = true;
            self.claims[1] = self.calc_max_id(1);
        } else {
            self.claims[1] = 0;
        }

        (meip, seip)
    }

    /// Determines if a context has any pending interrupt above its threshold.
    fn has_qualified_irq(&self, ctx: usize) -> bool {
        let threshold = self.thresholds[ctx];
        let num_words = std::cmp::min(self.pending.len(), self.enables[ctx].len());

        for word in 0..num_words {
            let active = self.pending[word] & self.enables[ctx][word];
            if active == 0 {
                continue;
            }
            for bit in 0..32 {
                let irq_id = word * 32 + bit;
                if irq_id == 0 {
                    continue; // IRQ 0 is reserved
                }
                if (active & (1 << bit)) != 0 && irq_id < self.priorities.len() {
                    if self.priorities[irq_id] > threshold {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Calculates the ID of the highest priority pending interrupt for a context.
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
}

impl Device for Plic {
    /// Returns the device name.
    fn name(&self) -> &str {
        "PLIC"
    }
    /// Returns the address range (Base, Size).
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x4000000)
    }

    /// Reads a word (32-bit) from the device.
    ///
    /// Handles reads from Priority, Pending, Enable, Threshold, and Claim registers.
    fn read_u32(&mut self, offset: u64) -> u32 {
        #[allow(clippy::absurd_extreme_comparisons)]
        if offset >= PLIC_PRIORITY_BASE && offset < PLIC_PENDING_BASE {
            let idx = (offset - PLIC_PRIORITY_BASE) as usize / 4;
            if idx < self.priorities.len() {
                return self.priorities[idx];
            }
        } else if offset >= PLIC_PENDING_BASE && offset < PLIC_ENABLE_BASE {
            let idx = (offset - PLIC_PENDING_BASE) as usize / 4;
            if idx < self.pending.len() {
                return self.pending[idx];
            }
        } else if offset >= PLIC_ENABLE_BASE && offset < PLIC_CONTEXT_BASE {
            let rel = (offset - PLIC_ENABLE_BASE) as usize;
            let ctx = rel / 0x80;
            let word_idx = (rel % 0x80) / 4;
            if ctx < NUM_CONTEXTS && word_idx < ENABLE_WORDS_PER_CONTEXT {
                return self.enables[ctx][word_idx];
            }
        } else if offset >= PLIC_CONTEXT_BASE {
            let ctx = (offset - PLIC_CONTEXT_BASE) as usize / 0x1000;
            let reg = offset & 0xFFF;
            if ctx < 2 {
                if reg == 0 {
                    return self.thresholds[ctx];
                }
                if reg == 4 {
                    let claim = self.claims[ctx];
                    if claim > 0 {
                        let idx = claim as usize / 32;
                        let bit = 1 << (claim % 32);
                        self.pending[idx] &= !bit;
                    }
                    return claim;
                }
            }
        }
        0
    }

    /// Writes a word (32-bit) to the device.
    ///
    /// Handles writes to Priority, Enable, Threshold, and Complete (Claim) registers.
    fn write_u32(&mut self, offset: u64, val: u32) {
        #[allow(clippy::absurd_extreme_comparisons)]
        if offset >= PLIC_PRIORITY_BASE && offset < PLIC_PENDING_BASE {
            let idx = (offset - PLIC_PRIORITY_BASE) as usize / 4;
            if idx < self.priorities.len() {
                self.priorities[idx] = val;
            }
        } else if offset >= PLIC_ENABLE_BASE && offset < PLIC_CONTEXT_BASE {
            let rel = (offset - PLIC_ENABLE_BASE) as usize;
            let ctx = rel / 0x80;
            let word_idx = (rel % 0x80) / 4;
            if ctx < NUM_CONTEXTS && word_idx < ENABLE_WORDS_PER_CONTEXT {
                self.enables[ctx][word_idx] = val;
            }
        } else if offset >= PLIC_CONTEXT_BASE {
            let ctx = (offset - PLIC_CONTEXT_BASE) as usize / 0x1000;
            let reg = offset & 0xFFF;
            if ctx < 2 {
                if reg == 0 {
                    self.thresholds[ctx] = val;
                }
                if reg == 4 {
                    self.claims[ctx] = 0;
                }
            }
        }
    }

    /// Reads a byte (delegates to read_u32).
    fn read_u8(&mut self, offset: u64) -> u8 {
        (self.read_u32(offset & !3) >> ((offset & 3) * 8)) as u8
    }
    /// Reads a half-word (delegates to read_u32).
    fn read_u16(&mut self, offset: u64) -> u16 {
        (self.read_u32(offset & !3) >> ((offset & 3) * 8)) as u16
    }
    /// Reads a double-word (delegates to read_u32).
    fn read_u64(&mut self, offset: u64) -> u64 {
        self.read_u32(offset) as u64
    }

    /// Writes a byte (delegates to write_u32).
    fn write_u8(&mut self, offset: u64, val: u8) {
        self.write_u32(offset & !3, val as u32);
    }
    /// Writes a half-word (delegates to write_u32).
    fn write_u16(&mut self, offset: u64, val: u16) {
        self.write_u32(offset & !3, val as u32);
    }
    /// Writes a double-word (delegates to write_u32).
    fn write_u64(&mut self, offset: u64, val: u64) {
        self.write_u32(offset, val as u32);
    }

    /// Advances the device state.
    ///
    /// Checks for pending interrupts and returns true if any are asserted.
    fn tick(&mut self) -> bool {
        let (meip, seip) = self.check_interrupts();
        meip || seip
    }

    /// Returns a mutable reference to the PLIC if this device is one.
    fn as_plic_mut(&mut self) -> Option<&mut Plic> {
        Some(self)
    }
}
