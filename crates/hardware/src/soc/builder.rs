//! System-on-Chip construction and top-level `System` type.
//!
//! This module builds the complete SoC from configuration. It performs:
//! 1. **Bus setup:** Creates the interconnect with configured width and latency.
//! 2. **Device registration:** Instantiates RAM, UART, VirtIO disk, CLINT, PLIC, SysCon, and RTC.
//! 3. **Memory controller:** Selects simple or DRAM controller based on config.
//! 4. **Binary loading:** Optionally loads a disk image from path and kernel via `load_binary_at`.

use crate::config::{Config, MemoryController as MemControllerType};
use crate::soc::devices::{Clint, GoldfishRtc, Plic, SysCon, Uart, VirtioBlock};
use crate::soc::interconnect::Bus;
use crate::soc::memory::Memory;
use crate::soc::memory::buffer::DramBuffer;
use crate::soc::memory::controller::{DramController, MemoryController, SimpleController};
use std::fs;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

/// Top-level system instance containing the bus, memory controller, and exit flag.
///
/// Holds the interconnect (`Bus`), the main memory controller (for DRAM/simple timing),
/// and an atomic exit request value used by devices (e.g., SysCon) to signal shutdown.
pub struct System {
    /// System interconnect; routes accesses to RAM and MMIO devices.
    pub bus: Bus,
    /// Main memory controller (boxed for dynamic dispatch; `Send + Sync` for multi-threaded simulation).
    pub mem_controller: Box<dyn MemoryController + Send + Sync>,
    /// Atomic exit code: when not `u64::MAX`, simulation should stop and use this as exit code.
    pub exit_request: Arc<AtomicU64>,
}

impl System {
    /// Builds a new system from configuration and optional disk image path.
    ///
    /// Creates the bus, RAM, UART, VirtIO disk (loading `disk_path` if non-empty), CLINT, PLIC,
    /// SysCon, and Goldfish RTC. The memory controller is chosen from `config.memory.controller`.
    ///
    /// # Arguments
    ///
    /// * `config` - Simulator configuration (system, memory, etc.).
    /// * `disk_path` - Path to disk image file; if non-empty and readable, data is loaded into VirtIO.
    ///
    /// # Returns
    ///
    /// A fully constructed `System` ready for simulation.
    pub fn new(config: &Config, disk_path: &str) -> Self {
        let mut bus = Bus::new(config.system.bus_width, config.system.bus_latency);
        let exit_request = Arc::new(AtomicU64::new(u64::MAX));

        let ram_base = config.system.ram_base;
        let ram_size = config.memory.ram_size;
        let ram_buffer = Arc::new(DramBuffer::new(ram_size));
        let mem = Memory::new(ram_buffer.clone(), ram_base);

        let uart_base = config.system.uart_base;
        let uart = Uart::new(uart_base, config.system.uart_to_stderr);

        let clint_addr = config.system.clint_base;
        let clint = Clint::new(clint_addr, config.system.clint_divider);

        let plic_addr = 0x0c00_0000;
        let plic = Plic::new(plic_addr);

        let disk_base = config.system.disk_base;
        let mut disk = VirtioBlock::new(disk_base, ram_base, ram_buffer);
        if !disk_path.is_empty() {
            if let Ok(disk_data) = fs::read(disk_path) {
                if !disk_data.is_empty() {
                    disk.load(disk_data);
                }
            }
        }

        let syscon_addr = config.system.syscon_base;
        let syscon = SysCon::new(syscon_addr, exit_request.clone());

        let rtc = GoldfishRtc::new(0x101000);

        bus.add_device(Box::new(mem));
        bus.add_device(Box::new(uart));
        bus.add_device(Box::new(disk));
        bus.add_device(Box::new(clint));
        bus.add_device(Box::new(plic));
        bus.add_device(Box::new(syscon));
        bus.add_device(Box::new(rtc));

        let mem_controller: Box<dyn MemoryController + Send + Sync> = match config.memory.controller
        {
            MemControllerType::Dram => Box::new(DramController::new(
                config.memory.t_cas,
                config.memory.t_ras,
                config.memory.t_pre,
            )),
            MemControllerType::Simple => {
                Box::new(SimpleController::new(config.memory.row_miss_latency))
            }
        };

        Self {
            bus,
            mem_controller,
            exit_request,
        }
    }

    /// Loads a binary into memory at the given physical address.
    ///
    /// # Arguments
    ///
    /// * `data` - Raw bytes to write.
    /// * `addr` - Physical base address for the write.
    pub fn load_binary_at(&mut self, data: &[u8], addr: u64) {
        self.bus.load_binary_at(data, addr);
    }

    /// Advances all devices by one tick; returns (timer_irq, meip, seip).
    ///
    /// # Returns
    ///
    /// A tuple of (machine timer IRQ active, machine external IRQ pending, supervisor external IRQ pending).
    pub fn tick(&mut self) -> (bool, bool, bool) {
        self.bus.tick()
    }

    /// Returns the requested exit code if a device has requested shutdown.
    ///
    /// # Returns
    ///
    /// `Some(exit_code)` if exit was requested, otherwise `None`.
    pub fn check_exit(&self) -> Option<u64> {
        let val = self.exit_request.load(std::sync::atomic::Ordering::Relaxed);
        if val != u64::MAX { Some(val) } else { None }
    }

    /// Checks whether the kernel has signaled panic via UART (e.g., for test harnesses).
    ///
    /// # Returns
    ///
    /// `true` if a kernel panic was detected, otherwise `false`.
    pub fn check_kernel_panic(&mut self) -> bool {
        self.bus.check_kernel_panic()
    }
}
