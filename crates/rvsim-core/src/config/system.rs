//! The system's address map and devices.

use super::defaults;
use crate::config::Console;
use serde::{Deserialize, Serialize};

/// A kind of MMIO device, named as configs, checkpoints and statistics
/// spell it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    /// The 16550 UART.
    #[serde(rename = "UART0")]
    Uart,
    /// The `VirtIO` block device.
    #[serde(rename = "VirtIO-Blk")]
    VirtioBlk,
    /// The core-local interruptor.
    #[serde(rename = "CLINT")]
    Clint,
    /// The platform-level interrupt controller.
    #[serde(rename = "PLIC")]
    Plic,
    /// The power and reset controller.
    #[serde(rename = "SysCon")]
    SysCon,
    /// The Goldfish real-time clock.
    #[serde(rename = "GoldfishRTC")]
    GoldfishRtc,
    /// The host-target interface.
    #[serde(rename = "HTIF")]
    Htif,
    /// A test's stand-in device.
    #[cfg(test)]
    Mock,
}

impl DeviceKind {
    /// Every kind.
    pub const ALL: [Self; 7] = [
        Self::Uart,
        Self::VirtioBlk,
        Self::Clint,
        Self::Plic,
        Self::SysCon,
        Self::GoldfishRtc,
        Self::Htif,
    ];

    /// The name configs, checkpoints and statistics use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uart => "UART0",
            Self::VirtioBlk => "VirtIO-Blk",
            Self::Clint => "CLINT",
            Self::Plic => "PLIC",
            Self::SysCon => "SysCon",
            Self::GoldfishRtc => "GoldfishRTC",
            Self::Htif => "HTIF",
            #[cfg(test)]
            Self::Mock => "Mock",
        }
    }
}

/// A device's register window in the physical address map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceRegion {
    /// The device's name, as its statistics and latency overrides spell it.
    pub name: &'static str,
    /// The window's first address.
    pub base: u64,
    /// The window's length in bytes.
    pub size: u64,
}

impl DeviceRegion {
    /// One past the window's last address.
    #[must_use]
    pub const fn end(self) -> u64 {
        self.base.saturating_add(self.size)
    }

    /// True when the two windows share an address.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.base < other.end() && other.base < self.end()
    }
}

/// The UART's register window in bytes.
pub const UART_REGION_BYTES: u64 = 0x100;
/// The `VirtIO` block device's register window in bytes.
pub const VIRTIO_REGION_BYTES: u64 = 0x1000;
/// The CLINT's register window in bytes.
pub const CLINT_REGION_BYTES: u64 = 0x1_0000;
/// The PLIC's fixed base address.
pub const PLIC_BASE: u64 = 0x0c00_0000;
/// The PLIC's register window in bytes.
pub const PLIC_REGION_BYTES: u64 = 0x400_0000;
/// The system controller's register window in bytes.
pub const SYSCON_REGION_BYTES: u64 = 0x1000;
/// The Goldfish RTC's fixed base address.
pub const RTC_BASE: u64 = 0x10_1000;
/// The Goldfish RTC's register window in bytes.
pub const RTC_REGION_BYTES: u64 = 0x1000;

/// How fast the CLINT's `mtime` advances.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum Timer {
    /// At `timebase_hz`, the rate the device tree reports, so guest time
    /// keeps pace with the modelled clock.
    #[default]
    RealTime,
    /// Every cycle, while the device tree still reports `timebase_hz`:
    /// guest time runs `cpu_clock_hz / timebase_hz` times fast, which
    /// shortens a boot's sleeps but floods measurements with timer
    /// interrupts.
    EveryCycle,
}

/// System memory map and bus configuration.
///
/// Defines memory-mapped I/O base addresses, RAM configuration,
/// and system bus parameters.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct SystemConfig {
    /// UART MMIO base address
    pub uart_base: u64,

    /// virtio disk MMIO base address
    pub disk_base: u64,

    /// Main RAM base address
    pub ram_base: u64,

    /// CLINT (timer) MMIO base address
    pub clint_base: u64,

    /// Syscon (power control) MMIO base address
    pub syscon_base: u64,

    /// Where a kernel image is loaded, from RAM base. With `fw_jump`
    /// firmware it must be where that firmware was built to jump.
    pub kernel_offset: u64,

    /// System bus width in bytes
    pub bus_width: u64,

    /// System bus latency in cycles
    pub bus_latency: u64,

    /// How fast the CLINT's `mtime` advances.
    pub timer: Timer,

    /// The timer frequency the device tree reports to the guest, in Hz.
    pub timebase_hz: u32,

    /// Core clock in MHz.
    pub cpu_clock_mhz: u64,

    /// Wall-clock time the RTC reports at cycle zero, in seconds since the
    /// Unix epoch.
    pub rtc_epoch_seconds: u64,

    /// Where the UART console reads and writes.
    pub console: Console,

    /// Time every device takes to answer a register access, in nanoseconds
    /// (gem5's `pio_latency`).
    pub device_latency_ns: u64,

    /// Per-device access latency in nanoseconds, by device name (`UART0`,
    /// `CLINT`, `PLIC`, `VirtIO-Blk`, `SysCon`, `GoldfishRTC`, `HTIF`),
    /// overriding `device_latency_ns`.
    pub device_latency_ns_overrides: std::collections::HashMap<DeviceKind, u64>,

    /// HTIF tohost address (0 = disabled). When non-zero, an HTIF device is
    /// registered at this address to intercept riscv-tests pass/fail writes.
    pub tohost_addr: u64,

    /// Number of harts in the simulated `SoC`. Default 1 (single-hart).
    pub hart_count: usize,
}

impl SystemConfig {
    /// Every device's register window but HTIF's, which a program's
    /// `tohost` places inside RAM.
    #[must_use]
    pub const fn device_regions(&self) -> [DeviceRegion; 6] {
        [
            DeviceRegion {
                name: DeviceKind::Uart.name(),
                base: self.uart_base,
                size: UART_REGION_BYTES,
            },
            DeviceRegion {
                name: DeviceKind::VirtioBlk.name(),
                base: self.disk_base,
                size: VIRTIO_REGION_BYTES,
            },
            DeviceRegion {
                name: DeviceKind::Clint.name(),
                base: self.clint_base,
                size: CLINT_REGION_BYTES,
            },
            DeviceRegion {
                name: DeviceKind::Plic.name(),
                base: PLIC_BASE,
                size: PLIC_REGION_BYTES,
            },
            DeviceRegion {
                name: DeviceKind::SysCon.name(),
                base: self.syscon_base,
                size: SYSCON_REGION_BYTES,
            },
            DeviceRegion {
                name: DeviceKind::GoldfishRtc.name(),
                base: RTC_BASE,
                size: RTC_REGION_BYTES,
            },
        ]
    }

    /// Cycles a device takes to answer a register access unless overridden.
    #[must_use]
    pub const fn default_device_access_cycles(&self) -> u64 {
        self.ns_to_cycles(self.device_latency_ns)
    }

    /// Cycles a device of `kind` takes to answer a register access.
    #[must_use]
    pub fn device_access_cycles(&self, kind: DeviceKind) -> u64 {
        self.device_latency_ns_overrides
            .get(&kind)
            .map_or_else(|| self.default_device_access_cycles(), |&ns| self.ns_to_cycles(ns))
    }

    /// The core clock in Hz, which SE mode's time and every device's
    /// latency are measured against.
    #[must_use]
    pub const fn cpu_clock_hz(&self) -> u64 {
        self.cpu_clock_mhz * 1_000_000
    }

    /// Cycles between ticks of the CLINT's `mtime`.
    #[must_use]
    pub const fn clint_divider(&self) -> u64 {
        match self.timer {
            Timer::RealTime => self.cpu_clock_hz() / self.timebase_hz as u64,
            Timer::EveryCycle => 1,
        }
    }

    /// Core cycles in `ns` nanoseconds.
    const fn ns_to_cycles(&self, ns: u64) -> u64 {
        ns * self.cpu_clock_mhz / 1000
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
            kernel_offset: defaults::KERNEL_OFFSET,
            bus_width: defaults::BUS_WIDTH,
            bus_latency: defaults::BUS_LATENCY,
            timer: Timer::default(),
            timebase_hz: defaults::TIMEBASE_HZ,
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
