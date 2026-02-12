//! System interconnect (bus) for memory and MMIO access.
//!
//! This module implements the bus that routes physical address accesses to devices. It provides:
//! 1. **Device registration:** Devices are added by address range and sorted for lookup.
//! 2. **Access routing:** Read/write by address with last-device hint for throughput.
//! 3. **Tick and IRQ:** Each device is ticked; PLIC aggregates IRQs for timer and external.
//! 4. **Load and RAM pointer:** Binary loading and raw RAM pointer for CPU DMA-style access.

use super::devices::Device;

/// System bus connecting CPU and devices; routes accesses by physical address.
///
/// Holds a sorted list of devices (RAM, UART, disk, CLINT, PLIC, etc.), bus width and latency
/// for transfer time calculation, and indices for fast RAM/UART/CLINT lookup.
pub struct Bus {
    /// Registered MMIO and memory devices (boxed for dynamic dispatch; `Send + Sync` for thread safety).
    devices: Vec<Box<dyn Device + Send + Sync>>,
    /// Bus width in bytes (e.g., 8 for 64-bit); used to compute transfer cycles.
    pub width_bytes: u64,
    /// Base latency in cycles per transaction.
    pub latency_cycles: u64,
    last_device_idx: usize,
    ram_idx: Option<usize>,
    uart_idx: Option<usize>,
}

impl Bus {
    /// Creates a new bus with the given width and latency.
    ///
    /// # Arguments
    ///
    /// * `width_bytes` - Transfer width in bytes (e.g., 8).
    /// * `latency_cycles` - Base cycles per transaction.
    ///
    /// # Returns
    ///
    /// An empty bus with no devices; add devices with `add_device`.
    pub fn new(width_bytes: u64, latency_cycles: u64) -> Self {
        Self {
            devices: Vec::new(),
            width_bytes,
            latency_cycles,
            last_device_idx: 0,
            ram_idx: None,
            uart_idx: None,
        }
    }

    /// Registers a device on the bus; devices are sorted by base address for lookup.
    ///
    /// # Arguments
    ///
    /// * `dev` - The device to add (must implement `Device` and be `Send + Sync`).
    pub fn add_device(&mut self, dev: Box<dyn Device + Send + Sync>) {
        self.devices.push(dev);
        self.devices.sort_by_key(|d| d.address_range().0);
        self.ram_idx = self.devices.iter().position(|d| d.name() == "DRAM");
        self.uart_idx = self.devices.iter().position(|d| d.name() == "UART0");
        self.last_device_idx = 0;
    }

    /// Returns the number of cycles to transfer the given number of bytes on this bus.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Number of bytes to transfer.
    ///
    /// # Returns
    ///
    /// Cycles = base latency plus ceiling(bytes / width_bytes) transfers.
    pub fn calculate_transit_time(&self, bytes: usize) -> u64 {
        let transfers = (bytes as u64 + self.width_bytes - 1) / self.width_bytes;
        self.latency_cycles + transfers
    }

    /// Writes a binary blob into memory at the given physical address.
    ///
    /// If a device claims the range, writes via that device; otherwise falls back to byte-by-byte write.
    ///
    /// # Arguments
    ///
    /// * `data` - Bytes to write.
    /// * `addr` - Physical base address.
    pub fn load_binary_at(&mut self, data: &[u8], addr: u64) {
        if let Some((dev, offset)) = self.find_device(addr) {
            let (_, size) = dev.address_range();
            if offset + (data.len() as u64) <= size {
                dev.write_bytes(offset, data);
                return;
            }
        }
        for (i, byte) in data.iter().enumerate() {
            self.write_u8(addr + i as u64, *byte);
        }
    }

    /// Returns whether the given physical address is backed by any device (e.g., RAM or MMIO).
    ///
    /// # Arguments
    ///
    /// * `paddr` - Physical address to check.
    ///
    /// # Returns
    ///
    /// `true` if some device's range contains `paddr`.
    pub fn is_valid_address(&self, paddr: u64) -> bool {
        if let Some(idx) = self.ram_idx {
            let (start, size) = self.devices[idx].address_range();
            if paddr >= start && paddr < start + size {
                return true;
            }
        }
        for dev in &self.devices {
            let (start, size) = dev.address_range();
            if paddr >= start && paddr < start + size {
                return true;
            }
        }
        false
    }

    /// Advances all devices by one tick and updates PLIC; returns IRQ flags.
    ///
    /// # Returns
    ///
    /// (timer_irq, meip, seip) for machine timer, machine external, and supervisor external interrupt.
    pub fn tick(&mut self) -> (bool, bool, bool) {
        let mut timer_irq = false;
        let mut active_irqs = 0u64;

        for i in 0..self.devices.len() {
            let dev = &mut self.devices[i];
            if dev.tick() {
                if let Some(id) = dev.get_irq_id() {
                    if id < 64 {
                        active_irqs |= 1 << id;
                    }
                }
                if dev.name() == "CLINT" {
                    timer_irq = true;
                }
            }
        }

        let (meip, seip) = if let Some(plic) = self.find_plic() {
            plic.update_irqs(active_irqs);
            plic.check_interrupts()
        } else {
            (false, false)
        };

        (timer_irq, meip, seip)
    }

    /// Returns whether the UART device has detected a kernel panic pattern (for test harnesses).
    ///
    /// # Returns
    ///
    /// `true` if kernel panic was detected.
    pub fn check_kernel_panic(&mut self) -> bool {
        if let Some(idx) = self.uart_idx {
            if idx < self.devices.len() {
                if let Some(uart) = self.devices[idx].as_uart_mut() {
                    return uart.check_kernel_panic();
                }
            }
        }
        false
    }

    /// Returns a raw pointer and (base, end) for the RAM region if present.
    ///
    /// Used by the CPU or loader for direct memory access (e.g., instruction fetch, DMA).
    ///
    /// # Returns
    ///
    /// `Some((ptr, base, end))` for RAM, or `None` if no RAM device is registered.
    pub fn get_ram_info(&mut self) -> Option<(*mut u8, u64, u64)> {
        if let Some(idx) = self.ram_idx {
            if let Some(mem) = self.devices[idx].as_memory_mut() {
                let (base, size) = mem.address_range();
                return Some((mem.as_mut_ptr(), base, base + size));
            }
        }
        None
    }

    fn find_plic(&mut self) -> Option<&mut crate::soc::devices::Plic> {
        for dev in &mut self.devices {
            if let Some(plic) = dev.as_plic_mut() {
                return Some(plic);
            }
        }
        None
    }

    fn find_device(&mut self, paddr: u64) -> Option<(&mut Box<dyn Device + Send + Sync>, u64)> {
        if self.last_device_idx < self.devices.len() {
            let (start, size) = self.devices[self.last_device_idx].address_range();
            if paddr >= start && paddr < start + size {
                return Some((&mut self.devices[self.last_device_idx], paddr - start));
            }
        }

        if let Some(idx) = self.ram_idx {
            let (start, size) = self.devices[idx].address_range();
            if paddr >= start && paddr < start + size {
                self.last_device_idx = idx;
                return Some((&mut self.devices[idx], paddr - start));
            }
        }

        for (i, dev) in self.devices.iter_mut().enumerate() {
            let (start, size) = dev.address_range();
            if paddr >= start && paddr < start + size {
                self.last_device_idx = i;
                return Some((dev, paddr - start));
            }
        }
        None
    }

    /// Reads one byte at the given physical address; returns 0 if no device claims the address.
    pub fn read_u8(&mut self, paddr: u64) -> u8 {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.read_u8(offset)
        } else {
            0
        }
    }
    /// Reads two bytes (little-endian) at the given physical address; returns 0 if unclaimed.
    pub fn read_u16(&mut self, paddr: u64) -> u16 {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.read_u16(offset)
        } else {
            0
        }
    }
    /// Reads four bytes (little-endian) at the given physical address; returns 0 if unclaimed.
    pub fn read_u32(&mut self, paddr: u64) -> u32 {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.read_u32(offset)
        } else {
            0
        }
    }
    /// Reads eight bytes (little-endian) at the given physical address; returns 0 if unclaimed.
    pub fn read_u64(&mut self, paddr: u64) -> u64 {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.read_u64(offset)
        } else {
            0
        }
    }
    /// Writes one byte at the given physical address; no-op if no device claims it.
    pub fn write_u8(&mut self, paddr: u64, val: u8) {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.write_u8(offset, val);
        }
    }
    /// Writes two bytes (little-endian) at the given physical address; no-op if unclaimed.
    pub fn write_u16(&mut self, paddr: u64, val: u16) {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.write_u16(offset, val);
        }
    }
    /// Writes four bytes (little-endian) at the given physical address; no-op if unclaimed.
    pub fn write_u32(&mut self, paddr: u64, val: u32) {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.write_u32(offset, val);
        }
    }
    /// Writes eight bytes (little-endian) at the given physical address; no-op if unclaimed.
    pub fn write_u64(&mut self, paddr: u64, val: u64) {
        if let Some((dev, offset)) = self.find_device(paddr) {
            dev.write_u64(offset, val);
        }
    }
}
