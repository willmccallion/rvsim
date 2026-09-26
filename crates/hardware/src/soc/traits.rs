//! `Device` trait implemented by all bus-attached MMIO components.
//!
//! Data-bearing operations (reads / writes) happen via the
//! [`Handle`](crate::sim::handle::Handle) trait, which every device implements.
//! `Device` itself describes the device's location on the bus, its lifecycle
//! tick, and type-specific upcasts that the bus uses for IRQ aggregation.

use crate::common::{IrqId, PhysAddr};
use crate::sim::handle::Handle;
use crate::soc::devices::{Clint, Plic, Uart};

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

    /// RAM ranges the device wrote by DMA since the last call, so the
    /// system can publish them like any other write (reservations, the
    /// write log).
    fn take_dma_writes(&mut self) -> Vec<(PhysAddr, usize)> {
        Vec::new()
    }
}
