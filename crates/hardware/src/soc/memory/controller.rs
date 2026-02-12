//! Memory controller implementations for latency modeling.
//!
//! This module provides:
//! 1. **SimpleController:** Fixed latency per access (no row-buffer modeling).
//! 2. **DramController:** Row-buffer-aware latency (CAS, RAS, precharge) for DRAM-style timing.
//!
//! Controllers are `Send + Sync` for use with the Python bindings and multi-threaded simulation.

/// Trait for memory controller implementations that report access latency in cycles.
///
/// Implementors must be `Send + Sync` for thread-safe use with the bus and Python bindings.
pub trait MemoryController: Send + Sync {
    /// Returns the number of cycles required for an access to the given address.
    ///
    /// # Arguments
    ///
    /// * `addr` - Physical address being accessed (may be used for row-buffer modeling).
    ///
    /// # Returns
    ///
    /// Latency in simulation cycles.
    fn access_latency(&mut self, addr: u64) -> u64;
}

/// Fixed-latency memory controller; every access takes the same number of cycles.
pub struct SimpleController {
    latency: u64,
}

impl SimpleController {
    /// Creates a simple controller with the given fixed latency in cycles.
    ///
    /// # Arguments
    ///
    /// * `latency` - Cycles per access.
    ///
    /// # Returns
    ///
    /// A new `SimpleController`.
    pub fn new(latency: u64) -> Self {
        Self { latency }
    }
}

impl MemoryController for SimpleController {
    fn access_latency(&mut self, _addr: u64) -> u64 {
        self.latency
    }
}

/// DRAM-style controller with row buffer; models CAS, RAS, and precharge latencies.
pub struct DramController {
    last_row: Option<u64>,
    t_cas: u64,
    t_ras: u64,
    t_pre: u64,
    row_mask: u64,
}

impl DramController {
    /// Creates a DRAM controller with the given timing parameters (in cycles).
    ///
    /// # Arguments
    ///
    /// * `t_cas` - Column access strobe latency.
    /// * `t_ras` - Row access strobe latency.
    /// * `t_pre` - Precharge latency.
    ///
    /// # Returns
    ///
    /// A new `DramController` with no row currently open.
    pub fn new(t_cas: u64, t_ras: u64, t_pre: u64) -> Self {
        Self {
            last_row: None,
            t_cas,
            t_ras,
            t_pre,
            row_mask: !2047,
        }
    }
}

impl MemoryController for DramController {
    fn access_latency(&mut self, addr: u64) -> u64 {
        let row = addr & self.row_mask;
        match self.last_row {
            Some(open_row) if open_row == row => self.t_cas,
            Some(_) => {
                self.last_row = Some(row);
                self.t_pre + self.t_ras + self.t_cas
            }
            None => {
                self.last_row = Some(row);
                self.t_ras + self.t_cas
            }
        }
    }
}
