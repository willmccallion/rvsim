//! Core Local Interruptor (CLINT).
//!
//! The CLINT block holds memory-mapped control and status registers associated
//! with software and timer interrupts. It complies with the RISC-V Privileged
//! Specification.
//!
//! # Memory Map
//!
//! * `0x0000`: MSIP (Machine Software Interrupt Pending)
//! * `0x4000`: MTIMECMP (Machine Time Compare)
//! * `0xBFF8`: MTIME (Machine Time)

use crate::soc::devices::Device;

/// Offset for the Machine Software Interrupt Pending register.
const MSIP_OFFSET: u64 = 0x0000;
/// Offset for the Machine Time Compare register.
const MTIMECMP_OFFSET: u64 = 0x4000;
/// Offset for the Machine Time register.
const MTIME_OFFSET: u64 = 0xBFF8;

/// CLINT device structure.
pub struct Clint {
    /// Base physical address of the device.
    base_addr: u64,
    /// Current machine time counter.
    mtime: u64,
    /// Machine time compare register.
    mtimecmp: u64,
    /// Machine software interrupt pending register.
    msip: u32,
    /// Divider to scale CPU cycles to timer ticks.
    divider: u64,
    /// Internal counter for the divider.
    counter: u64,
}

impl Clint {
    /// Creates a new CLINT device.
    ///
    /// # Arguments
    ///
    /// * `base_addr` - The base physical address.
    /// * `divider` - The ratio of CPU cycles to timer ticks (e.g., 10 means timer increments every 10 cycles).
    pub fn new(base_addr: u64, divider: u64) -> Self {
        Self {
            base_addr,
            mtime: 0,
            mtimecmp: u64::MAX,
            msip: 0,
            divider: if divider == 0 { 1 } else { divider },
            counter: 0,
        }
    }
}

impl Device for Clint {
    /// Returns the device name.
    fn name(&self) -> &str {
        "CLINT"
    }

    /// Returns the address range (Base, Size).
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x10000)
    }

    /// Reads a byte from the device.
    ///
    /// Delegates to `read_u64` and extracts the appropriate byte.
    fn read_u8(&mut self, offset: u64) -> u8 {
        let val = self.read_u64(offset & !7);
        let shift = (offset & 7) * 8;
        ((val >> shift) & 0xFF) as u8
    }

    /// Reads a half-word (unimplemented, returns 0).
    fn read_u16(&mut self, _offset: u64) -> u16 {
        0
    }

    /// Reads a word (32-bit) from the device.
    ///
    /// Handles reads to MSIP, and the lower/upper halves of MTIME and MTIMECMP.
    fn read_u32(&mut self, offset: u64) -> u32 {
        match offset {
            MSIP_OFFSET => self.msip,
            MTIMECMP_OFFSET => self.mtimecmp as u32,
            val if val == MTIMECMP_OFFSET + 4 => (self.mtimecmp >> 32) as u32,
            MTIME_OFFSET => self.mtime as u32,
            val if val == MTIME_OFFSET + 4 => (self.mtime >> 32) as u32,
            _ => 0,
        }
    }

    /// Reads a double-word (64-bit) from the device.
    fn read_u64(&mut self, offset: u64) -> u64 {
        match offset {
            MSIP_OFFSET => self.msip as u64,
            MTIMECMP_OFFSET => self.mtimecmp,
            MTIME_OFFSET => self.mtime,
            _ => 0,
        }
    }

    /// Writes a byte (unimplemented).
    fn write_u8(&mut self, _offset: u64, _val: u8) {}
    /// Writes a half-word (unimplemented).
    fn write_u16(&mut self, _offset: u64, _val: u16) {}

    /// Writes a word (32-bit) to the device.
    ///
    /// Handles writes to MSIP, and the lower/upper halves of MTIME and MTIMECMP.
    fn write_u32(&mut self, offset: u64, val: u32) {
        match offset {
            MSIP_OFFSET => self.msip = val & 1,
            MTIMECMP_OFFSET => {
                self.mtimecmp = (self.mtimecmp & 0xFFFF_FFFF_0000_0000) | (val as u64)
            }
            o if o == MTIMECMP_OFFSET + 4 => {
                self.mtimecmp = (self.mtimecmp & 0x0000_0000_FFFF_FFFF) | ((val as u64) << 32)
            }
            MTIME_OFFSET => self.mtime = (self.mtime & 0xFFFF_FFFF_0000_0000) | (val as u64),
            o if o == MTIME_OFFSET + 4 => {
                self.mtime = (self.mtime & 0x0000_0000_FFFF_FFFF) | ((val as u64) << 32)
            }
            _ => {}
        }
    }

    /// Writes a double-word (64-bit) to the device.
    fn write_u64(&mut self, offset: u64, val: u64) {
        match offset {
            MSIP_OFFSET => self.msip = (val as u32) & 1,
            MTIMECMP_OFFSET => self.mtimecmp = val,
            MTIME_OFFSET => self.mtime = val,
            _ => {}
        }
    }

    /// Advances the device state by one cycle.
    ///
    /// Increments the `mtime` counter based on the configured divider.
    /// Returns `true` if an interrupt condition is met (timer or software).
    fn tick(&mut self) -> bool {
        self.counter += 1;
        if self.counter >= self.divider {
            self.mtime = self.mtime.wrapping_add(1);
            self.counter = 0;
        }

        self.mtime >= self.mtimecmp || (self.msip & 1) != 0
    }
}
