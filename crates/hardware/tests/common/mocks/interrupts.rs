/// Mock interrupt controller for test scenarios.
///
/// Tracks pending IRQ lines and claim/complete operations without
/// requiring a full PLIC device on the bus.
pub struct MockInterruptController {
    pending: u64,
    enabled: u64,
    threshold: u32,
}

impl MockInterruptController {
    pub fn new() -> Self {
        Self {
            pending: 0,
            enabled: 0,
            threshold: 0,
        }
    }

    /// Raise an interrupt line (0-63).
    pub fn raise(&mut self, irq: u32) {
        assert!(irq < 64, "IRQ id must be < 64");
        self.pending |= 1 << irq;
    }

    /// Clear an interrupt line.
    pub fn clear(&mut self, irq: u32) {
        assert!(irq < 64, "IRQ id must be < 64");
        self.pending &= !(1 << irq);
    }

    /// Enable an interrupt line.
    pub fn enable(&mut self, irq: u32) {
        assert!(irq < 64, "IRQ id must be < 64");
        self.enabled |= 1 << irq;
    }

    /// Set the priority threshold.
    pub fn set_threshold(&mut self, threshold: u32) {
        self.threshold = threshold;
    }

    /// Returns the highest-priority pending & enabled IRQ, or None.
    pub fn claim(&mut self) -> Option<u32> {
        let actionable = self.pending & self.enabled;
        if actionable == 0 {
            return None;
        }
        let irq = actionable.trailing_zeros();
        self.pending &= !(1 << irq);
        Some(irq)
    }

    /// Returns whether any interrupts are pending and enabled.
    pub fn is_pending(&self) -> bool {
        (self.pending & self.enabled) != 0
    }
}
