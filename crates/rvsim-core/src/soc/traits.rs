//! `Device` trait implemented by all bus-attached MMIO components.
//!
//! Data-bearing operations (reads / writes) happen via the
//! [`crate::sim::handle::Handle`] trait, which every device implements.
//! `Device` itself describes the device's location on the bus, its lifecycle
//! tick, and type-specific upcasts that the bus uses for IRQ aggregation.

use crate::common::IrqId;
use crate::sim::handle::Handle;
use crate::sim::memory::GlobalMemory;
use crate::soc::devices::{Clint, Plic, SimOp, Uart};

/// Trait for memory-mapped I/O devices attached to the system bus.
///
/// All data accesses go through the device's [`Handle`] impl; this trait only
/// describes layout (name + address range), per-cycle lifecycle (`tick`), and
/// device-class upcasts the bus uses for routing IRQs and panic detection.
pub trait Device: Handle + Send + Sync {
    /// Returns a short name for this device (e.g., `"UART0"`, `"CLINT"`).
    fn name(&self) -> &str;
    /// Returns (`base_address`, `size_in_bytes`) for this device's MMIO region.
    fn address_range(&self) -> (u64, u64);

    /// Advances device state by one cycle; returns `true` if an IRQ was raised
    /// (e.g., timer).
    fn tick(&mut self) -> bool {
        false
    }
    /// How many of the coming ticks would change nothing but the device's
    /// own clock, or `None` when it will stay that way until software
    /// touches it. The default says none, so a device skips no time until
    /// it implements [`Device::skip_ticks`].
    fn quiet_ticks(&self) -> Option<u64> {
        Some(0)
    }

    /// Advances the device by `ticks` ticks, at most [`Device::quiet_ticks`],
    /// leaving it exactly as ticking would.
    fn skip_ticks(&mut self, _ticks: u64) {}

    /// Returns the IRQ ID for this device if it can raise interrupts.
    fn get_irq_id(&self) -> Option<IrqId> {
        None
    }

    /// Returns a reference as `Clint` if this device is the CLINT.
    fn as_clint(&self) -> Option<&Clint> {
        None
    }
    /// Returns a mutable reference as `Clint` if this device is the CLINT.
    fn as_clint_mut(&mut self) -> Option<&mut Clint> {
        None
    }
    /// Returns a mutable reference as `Plic` if this device is the PLIC.
    fn as_plic_mut(&mut self) -> Option<&mut Plic> {
        None
    }
    /// Returns a mutable reference as `Uart` if this device is a UART.
    fn as_uart_mut(&mut self) -> Option<&mut Uart> {
        None
    }

    /// Requests the guest made of the simulator since the last call.
    fn take_sim_ops(&mut self) -> Vec<SimOp> {
        Vec::new()
    }

    /// Finishes the device's work in flight before a checkpoint, the way
    /// a pipeline drain writes its committed stores.
    fn drain(&mut self, _memory: &mut GlobalMemory) {}

    /// The device's architectural state for a checkpoint; `None` for a
    /// device with none.
    fn checkpoint(&self) -> Option<serde_json::Value> {
        None
    }

    /// Checks that [`Device::restore`] would take `state`, without
    /// changing anything, so a restore can fail before it starts.
    ///
    /// # Errors
    ///
    /// Describes why the state cannot be restored into this device.
    fn check_restore(&self, _state: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    /// Restores state a [`Device::checkpoint`] produced.
    ///
    /// # Errors
    ///
    /// Describes why the state cannot be restored into this device.
    fn restore(&mut self, _state: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }
}
