//! System Controller (`SysCon`).
//!
//! A simple memory-mapped device used to control system power and reset states.
//! It is primarily used by the kernel or test environment to gracefully exit
//! the simulation or trigger a reset.
//!
//! # Registers
//!
//! * `0x00`: Command Register (Write Only)
//!   * `0x5555`: Power Off
//!   * `0x7777`: Reset
//!   * `0x3333`: Failure/Panic

use crate::common::{PhysAddr, PhysRange};
use crate::config::DeviceKind;
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::AccessSize;
use crate::sim::packet::{MemOp, Packet, WriteData};
use crate::soc::devices::Device;
use crate::soc::exit::ExitSignal;
use crate::soc::traits::{MmioRegisters, serve_mmio};

/// `SysCon` device structure.
#[derive(Debug)]
pub struct SysCon {
    /// Base physical address of the device.
    base_addr: PhysAddr,
    /// Shared atomic flag to signal the simulation loop to exit.
    exit_signal: ExitSignal,
}

impl SysCon {
    /// The command that powers the machine off.
    pub const POWER_OFF: u32 = 0x5555;
    /// The command that resets the machine, which ends the run.
    pub const RESET: u32 = 0x7777;
    /// The command that reports a failure.
    pub const FAIL: u32 = 0x3333;

    /// Creates a new `SysCon` device.
    pub const fn new(base_addr: PhysAddr, exit_signal: ExitSignal) -> Self {
        Self { base_addr, exit_signal }
    }

    fn act_on_command(&self, val: u32) {
        match val {
            Self::POWER_OFF => {
                tracing::info!(target: "rvsim::syscon", "poweroff");
                self.exit_signal.signal(0);
            }
            Self::RESET => {
                tracing::info!(target: "rvsim::syscon", "reset, taken as exit");
                self.exit_signal.signal(0);
            }
            Self::FAIL => {
                tracing::info!(target: "rvsim::syscon", "failure");
                self.exit_signal.signal(1);
            }
            _ => {}
        }
    }
}

impl Handle for SysCon {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        serve_mmio(self, packet, source, ctx);
    }
}

impl MmioRegisters for SysCon {
    fn access(
        &mut self,
        offset: u64,
        size: AccessSize,
        op: MemOp,
        _ctx: &mut HandleCtx<'_>,
    ) -> u64 {
        // SysCon commands are 32-bit (0x5555 / 0x7777 / 0x3333). Sub-word
        // partial writes that don't reach a full register width are
        // discarded, mirroring real device behaviour and what tests rely on.
        if offset == 0
            && matches!(
                size,
                crate::sim::packet::AccessSize::B4 | crate::sim::packet::AccessSize::B8
            )
            && let MemOp::Write { data: WriteData::Small(val), .. } = op
        {
            self.act_on_command(val as u32);
        }
        0
    }
}

impl Device for SysCon {
    fn kind(&self) -> DeviceKind {
        DeviceKind::SysCon
    }

    fn region(&self) -> PhysRange {
        PhysRange::new(self.base_addr, crate::config::SYSCON_REGION_BYTES)
    }

    fn quiet_ticks(&self) -> Option<u64> {
        None
    }
}
