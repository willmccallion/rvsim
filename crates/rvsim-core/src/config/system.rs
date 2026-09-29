//! The system's address map and devices.

use super::defaults;
use crate::config::Console;
use serde::Deserialize;

/// System memory map and bus configuration.
///
/// Defines memory-mapped I/O base addresses, RAM configuration,
/// and system bus parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct SystemConfig {
    /// UART MMIO base address
    #[serde(default = "SystemConfig::default_uart_base")]
    pub uart_base: u64,

    /// virtio disk MMIO base address
    #[serde(default = "SystemConfig::default_disk_base")]
    pub disk_base: u64,

    /// Main RAM base address
    #[serde(default = "SystemConfig::default_ram_base")]
    pub ram_base: u64,

    /// CLINT (timer) MMIO base address
    #[serde(default = "SystemConfig::default_clint_base")]
    pub clint_base: u64,

    /// Syscon (power control) MMIO base address
    #[serde(default = "SystemConfig::default_syscon_base")]
    pub syscon_base: u64,

    /// Simulator control MMIO base address, which guest software writes
    /// to reset or dump the stats and to end the run
    #[serde(default = "SystemConfig::default_sim_control_base")]
    pub sim_control_base: u64,

    /// Kernel load offset from RAM base
    #[serde(default = "SystemConfig::default_kernel_offset")]
    pub kernel_offset: u64,

    /// System bus width in bytes
    #[serde(default = "SystemConfig::default_bus_width")]
    pub bus_width: u64,

    /// System bus latency in cycles
    #[serde(default = "SystemConfig::default_bus_latency")]
    pub bus_latency: u64,

    /// CLINT timer divider (mtime increments every N cycles)
    #[serde(default = "SystemConfig::default_clint_divider")]
    pub clint_divider: u64,

    /// Core clock in MHz.
    #[serde(default = "SystemConfig::default_cpu_clock_mhz")]
    pub cpu_clock_mhz: u64,

    /// Wall-clock time the RTC reports at cycle zero, in seconds since the
    /// Unix epoch.
    #[serde(default = "SystemConfig::default_rtc_epoch_seconds")]
    pub rtc_epoch_seconds: u64,

    /// Where the UART console reads and writes.
    #[serde(default)]
    pub console: Console,

    /// Time every device takes to answer a register access, in nanoseconds
    /// (gem5's `pio_latency`).
    #[serde(default = "SystemConfig::default_device_latency_ns")]
    pub device_latency_ns: u64,

    /// Per-device access latency in nanoseconds, by device name (`UART0`,
    /// `CLINT`, `PLIC`, `VirtIO-Blk`, `SysCon`, `GoldfishRTC`, `HTIF`),
    /// overriding `device_latency_ns`.
    #[serde(default)]
    pub device_latency_ns_overrides: std::collections::HashMap<String, u64>,

    /// HTIF tohost address (0 = disabled). When non-zero, an HTIF device is
    /// registered at this address to intercept riscv-tests pass/fail writes.
    #[serde(default)]
    pub tohost_addr: u64,

    /// Number of harts in the simulated `SoC`. Default 1 (single-hart).
    /// Multi-hart support lands later in Phase C; this knob exists from
    /// Phase A so config plumbing is in place when n>1 is enabled.
    #[serde(default = "SystemConfig::default_hart_count")]
    pub hart_count: usize,
}

impl SystemConfig {
    /// Returns the default UART MMIO base address.
    const fn default_uart_base() -> u64 {
        defaults::UART_BASE
    }

    /// Returns the default virtio disk MMIO base address.
    const fn default_disk_base() -> u64 {
        defaults::DISK_BASE
    }

    /// Returns the default RAM base address.
    const fn default_ram_base() -> u64 {
        defaults::RAM_BASE
    }

    /// Returns the default CLINT MMIO base address.
    const fn default_clint_base() -> u64 {
        defaults::CLINT_BASE
    }

    /// Returns the default system controller MMIO base address.
    const fn default_syscon_base() -> u64 {
        defaults::SYSCON_BASE
    }

    const fn default_sim_control_base() -> u64 {
        defaults::SIM_CONTROL_BASE
    }

    /// Returns the default kernel load offset from RAM base.
    const fn default_kernel_offset() -> u64 {
        defaults::KERNEL_OFFSET
    }

    /// Returns the default system bus width in bytes.
    const fn default_bus_width() -> u64 {
        defaults::BUS_WIDTH
    }

    /// Returns the default system bus latency in cycles.
    const fn default_bus_latency() -> u64 {
        defaults::BUS_LATENCY
    }

    /// Returns the default CLINT timer divider value.
    const fn default_clint_divider() -> u64 {
        defaults::CLINT_DIVIDER
    }

    const fn default_cpu_clock_mhz() -> u64 {
        defaults::CPU_CLOCK_MHZ
    }

    const fn default_device_latency_ns() -> u64 {
        defaults::DEVICE_LATENCY_NS
    }

    /// Cycles a device takes to answer a register access unless overridden.
    #[must_use]
    pub const fn default_device_access_cycles(&self) -> u64 {
        self.ns_to_cycles(self.device_latency_ns)
    }

    /// Cycles the device named `name` takes to answer a register access.
    #[must_use]
    pub fn device_access_cycles(&self, name: &str) -> u64 {
        self.device_latency_ns_overrides
            .get(name)
            .map_or_else(|| self.default_device_access_cycles(), |&ns| self.ns_to_cycles(ns))
    }

    /// Core cycles in `ns` nanoseconds.
    const fn ns_to_cycles(&self, ns: u64) -> u64 {
        ns * self.cpu_clock_mhz / 1000
    }

    const fn default_rtc_epoch_seconds() -> u64 {
        defaults::RTC_EPOCH_SECONDS
    }

    /// Returns the default hart count (1).
    const fn default_hart_count() -> usize {
        1
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            uart_base: defaults::UART_BASE,
            disk_base: defaults::DISK_BASE,
            ram_base: defaults::RAM_BASE,
            clint_base: defaults::CLINT_BASE,
            syscon_base: defaults::SYSCON_BASE,
            sim_control_base: defaults::SIM_CONTROL_BASE,
            kernel_offset: defaults::KERNEL_OFFSET,
            bus_width: defaults::BUS_WIDTH,
            bus_latency: defaults::BUS_LATENCY,
            clint_divider: defaults::CLINT_DIVIDER,
            cpu_clock_mhz: defaults::CPU_CLOCK_MHZ,
            rtc_epoch_seconds: defaults::RTC_EPOCH_SECONDS,
            console: Console::default(),
            device_latency_ns: defaults::DEVICE_LATENCY_NS,
            device_latency_ns_overrides: std::collections::HashMap::new(),
            tohost_addr: 0,
            hart_count: 1,
        }
    }
}
