//! VirtIO Block Device (MMIO).
//!
//! Implements a VirtIO block device over Memory-Mapped I/O (MMIO) for disk access.
//! Supports the legacy VirtIO interface required by the Linux kernel.

use crate::soc::devices::Device;
use crate::soc::memory::buffer::DramBuffer;
use std::convert::TryInto;
use std::sync::Arc;

/// VirtIO MMIO magic value register offset.
const REG_MAGIC: u64 = 0x00;

/// VirtIO MMIO version register offset.
const REG_VERSION: u64 = 0x04;

/// VirtIO MMIO device ID register offset.
const REG_DEVICE_ID: u64 = 0x08;

/// VirtIO MMIO vendor ID register offset.
const REG_VENDOR_ID: u64 = 0x0c;

/// VirtIO MMIO device features register offset.
const REG_DEVICE_FEATURES: u64 = 0x10;

/// VirtIO MMIO device features select register offset.
const REG_DEVICE_FEATURES_SEL: u64 = 0x14;

/// VirtIO MMIO driver features register offset.
const REG_DRIVER_FEATURES: u64 = 0x20;

/// VirtIO MMIO driver features select register offset.
const REG_DRIVER_FEATURES_SEL: u64 = 0x24;

/// VirtIO MMIO queue select register offset.
const REG_QUEUE_SEL: u64 = 0x30;

/// VirtIO MMIO queue maximum size register offset.
const REG_QUEUE_NUM_MAX: u64 = 0x34;

/// VirtIO MMIO queue size register offset.
const REG_QUEUE_NUM: u64 = 0x38;

/// VirtIO MMIO queue ready register offset.
const REG_QUEUE_READY: u64 = 0x44;

/// VirtIO MMIO queue notify register offset.
const REG_QUEUE_NOTIFY: u64 = 0x50;

/// VirtIO MMIO interrupt status register offset.
const REG_INTERRUPT_STATUS: u64 = 0x60;

/// VirtIO MMIO interrupt acknowledge register offset.
const REG_INTERRUPT_ACK: u64 = 0x64;

/// VirtIO MMIO device status register offset.
const REG_STATUS: u64 = 0x70;

/// VirtIO MMIO queue descriptor table address (low 32 bits) register offset.
const REG_QUEUE_DESC_LOW: u64 = 0x80;

/// VirtIO MMIO queue descriptor table address (high 32 bits) register offset.
const REG_QUEUE_DESC_HIGH: u64 = 0x84;

/// VirtIO MMIO queue available ring address (low 32 bits) register offset.
const REG_QUEUE_AVAIL_LOW: u64 = 0x90;

/// VirtIO MMIO queue available ring address (high 32 bits) register offset.
const REG_QUEUE_AVAIL_HIGH: u64 = 0x94;

/// VirtIO MMIO queue used ring address (low 32 bits) register offset.
const REG_QUEUE_USED_LOW: u64 = 0xa0;

/// VirtIO MMIO queue used ring address (high 32 bits) register offset.
const REG_QUEUE_USED_HIGH: u64 = 0xa4;

/// VirtIO MMIO configuration space base offset.
const REG_CONFIG_BASE: u64 = 0x100;

/// VirtIO MMIO magic value ("virt" in ASCII: 0x74726976).
const VIRTIO_MMIO_MAGIC_VALUE: u32 = 0x74726976;

/// VirtIO MMIO vendor ID value (QEMU vendor: 0x554d4551).
const VIRTIO_MMIO_VENDOR_ID_VALUE: u32 = 0x554d4551;

/// VirtIO MMIO device ID for block device (2).
const VIRTIO_MMIO_DEVICE_ID_VALUE: u32 = 2;

/// VirtIO specification version (2).
const VIRTIO_VERSION_VALUE: u32 = 2;

/// Maximum queue size supported by this device (16 entries).
const QUEUE_NUM_MAX_VALUE: u32 = 16;

/// Size of a virtqueue descriptor in bytes (16 bytes).
const DESC_SIZE: u64 = 16;

/// Offset of address field within descriptor (bytes 0-7).
const DESC_OFFSET_ADDR: u64 = 0;

/// Offset of length field within descriptor (bytes 8-11).
const DESC_OFFSET_LEN: u64 = 8;

/// Offset of flags field within descriptor (bytes 12-13).
const DESC_OFFSET_FLAGS: u64 = 12;

/// Offset of next descriptor index field within descriptor (bytes 14-15).
const DESC_OFFSET_NEXT: u64 = 14;

/// Virtqueue descriptor flag: indicates chained descriptors (more descriptors follow).
const VRING_DESC_F_NEXT: u16 = 1;

/// Virtqueue descriptor flag: indicates write-only descriptor (device writes to memory).
const VRING_DESC_F_WRITE: u16 = 2;

/// Disk sector size in bytes (512 bytes per sector).
const SECTOR_SIZE: u64 = 512;

/// VirtIO Block device structure.
///
/// Implements a memory-mapped block device compliant with the VirtIO specification.
/// It uses a shared DRAM buffer to perform DMA operations for reading and writing
/// disk sectors.
pub struct VirtioBlock {
    /// Base physical address of the device MMIO region.
    base_addr: u64,
    /// Base physical address of system RAM.
    ram_base: u64,
    /// Disk image data.
    disk_image: Vec<u8>,
    /// Shared reference to system RAM for DMA.
    ram: Arc<DramBuffer>,

    /// Device status register.
    status: u32,
    /// Configured queue size.
    queue_num: u32,
    /// Queue ready bit.
    queue_ready: u32,
    /// Queue notify register (triggers processing).
    queue_notify: u32,

    /// Queue Descriptor Table address (Low 32 bits).
    queue_desc_low: u32,
    /// Queue Descriptor Table address (High 32 bits).
    queue_desc_high: u32,
    /// Queue Available Ring address (Low 32 bits).
    queue_avail_low: u32,
    /// Queue Available Ring address (High 32 bits).
    queue_avail_high: u32,
    /// Queue Used Ring address (Low 32 bits).
    queue_used_low: u32,
    /// Queue Used Ring address (High 32 bits).
    queue_used_high: u32,

    /// Interrupt status register.
    interrupt_status: u32,
    /// Last processed available index.
    last_avail_idx: u16,

    /// Device features selection.
    device_features_sel: u32,
    /// Driver features selection.
    driver_features_sel: u32,
}

unsafe impl Send for VirtioBlock {}
unsafe impl Sync for VirtioBlock {}

impl VirtioBlock {
    /// Creates a new VirtIO Block device.
    ///
    /// # Arguments
    ///
    /// * `base_addr` - MMIO base address.
    /// * `ram_base` - System RAM base address.
    /// * `ram` - Shared DRAM buffer for DMA access.
    pub fn new(base_addr: u64, ram_base: u64, ram: Arc<DramBuffer>) -> Self {
        Self {
            base_addr,
            ram_base,
            disk_image: Vec::new(),
            ram,
            status: 0,
            queue_num: 0,
            queue_ready: 0,
            queue_notify: 0,
            queue_desc_low: 0,
            queue_desc_high: 0,
            queue_avail_low: 0,
            queue_avail_high: 0,
            queue_used_low: 0,
            queue_used_high: 0,
            interrupt_status: 0,
            last_avail_idx: 0,
            device_features_sel: 0,
            driver_features_sel: 0,
        }
    }

    /// Loads a disk image into the device.
    ///
    /// # Arguments
    ///
    /// * `data` - The raw bytes of the disk image.
    pub fn load(&mut self, data: Vec<u8>) {
        self.disk_image = data;
    }

    /// Performs a Direct Memory Access (DMA) read from system RAM.
    ///
    /// Reads `len` bytes from the physical address `addr`.
    ///
    /// # Arguments
    ///
    /// * `addr` - Physical address to read from.
    /// * `len` - Number of bytes to read.
    ///
    /// # Returns
    ///
    /// A vector containing the read bytes. Returns zeroed bytes if the address
    /// is out of bounds.
    fn dma_read(&self, addr: u64, len: usize) -> Vec<u8> {
        if addr < self.ram_base {
            return vec![0; len];
        }
        let offset = (addr - self.ram_base) as usize;

        if offset >= self.ram.len() || offset + len > self.ram.len() {
            return vec![0; len];
        }

        self.ram.read_slice(offset, len).to_vec()
    }

    /// Performs a Direct Memory Access (DMA) write to system RAM.
    ///
    /// Writes `data` to the physical address `addr`.
    ///
    /// # Arguments
    ///
    /// * `addr` - Physical address to write to.
    /// * `data` - Bytes to write.
    fn dma_write(&self, addr: u64, data: &[u8]) {
        if addr < self.ram_base {
            println!("[VirtIO] DMA Write Out of Bounds (Low): 0x{:x}", addr);
            return;
        }
        let offset = (addr - self.ram_base) as usize;

        if offset >= self.ram.len() || offset + data.len() > self.ram.len() {
            println!(
                "[VirtIO] DMA Write Out of Bounds (High): 0x{:x} (Size: {})",
                addr,
                data.len()
            );
            return;
        }

        self.ram.write_slice(offset, data);
    }

    /// Processes the VirtQueue.
    ///
    /// Reads descriptors from the Available Ring, executes the requests (Read/Write),
    /// and updates the Used Ring. This is triggered by a write to the Queue Notify register.
    fn process_queue(&mut self) {
        if self.queue_num == 0 {
            return;
        }

        let desc_addr = ((self.queue_desc_high as u64) << 32) | (self.queue_desc_low as u64);
        let avail_addr = ((self.queue_avail_high as u64) << 32) | (self.queue_avail_low as u64);
        let used_addr = ((self.queue_used_high as u64) << 32) | (self.queue_used_low as u64);

        let avail_idx = u16::from_le_bytes(self.dma_read(avail_addr + 2, 2).try_into().unwrap());

        while self.last_avail_idx != avail_idx {
            let ring_offset = 4 + (self.last_avail_idx as u64 % self.queue_num as u64) * 2;
            let head_idx = u16::from_le_bytes(
                self.dma_read(avail_addr + ring_offset, 2)
                    .try_into()
                    .unwrap(),
            );

            if head_idx as u32 >= self.queue_num {
                println!(
                    "[VirtIO] Error: Head descriptor index {} out of bounds (Queue Size {})",
                    head_idx, self.queue_num
                );
                self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
                continue;
            }

            let mut current_idx = head_idx;
            let mut descriptors = Vec::new();

            loop {
                if current_idx as u32 >= self.queue_num {
                    println!(
                        "[VirtIO] Error: Descriptor index {} out of bounds (Queue Size {})",
                        current_idx, self.queue_num
                    );
                    break;
                }

                let addr_offset = desc_addr + (current_idx as u64 * DESC_SIZE);
                let addr = u64::from_le_bytes(
                    self.dma_read(addr_offset + DESC_OFFSET_ADDR, 8)
                        .try_into()
                        .unwrap(),
                );
                let len = u32::from_le_bytes(
                    self.dma_read(addr_offset + DESC_OFFSET_LEN, 4)
                        .try_into()
                        .unwrap(),
                );
                let flags = u16::from_le_bytes(
                    self.dma_read(addr_offset + DESC_OFFSET_FLAGS, 2)
                        .try_into()
                        .unwrap(),
                );
                let next = u16::from_le_bytes(
                    self.dma_read(addr_offset + DESC_OFFSET_NEXT, 2)
                        .try_into()
                        .unwrap(),
                );

                descriptors.push((addr, len, flags));

                if (flags & VRING_DESC_F_NEXT) == 0 {
                    break;
                }
                current_idx = next;
            }

            let mut len_written = 0;
            if descriptors.len() >= 3 {
                let (h_addr, _, _) = descriptors[0];
                let header = self.dma_read(h_addr, 16);
                let type_val = u32::from_le_bytes(header[0..4].try_into().unwrap());
                let sector = u64::from_le_bytes(header[8..16].try_into().unwrap());
                let is_write = (type_val & 1) != 0;

                let (s_addr, _, _) = descriptors[descriptors.len() - 1];

                let sector_offset = (sector * SECTOR_SIZE) as usize;
                let mut current_offset = 0;

                if is_write {
                    let mut current_disk_offset = sector_offset;

                    for i in 1..descriptors.len() - 1 {
                        let (d_addr, d_len, _) = descriptors[i];

                        let data = self.dma_read(d_addr, d_len as usize);
                        if current_disk_offset + data.len() <= self.disk_image.len() {
                            self.disk_image[current_disk_offset..current_disk_offset + data.len()]
                                .copy_from_slice(&data);
                        }
                        current_disk_offset += d_len as usize;
                        len_written += d_len;
                    }
                } else {
                    for i in 1..descriptors.len() - 1 {
                        let (d_addr, d_len, d_flags) = descriptors[i];
                        if (d_flags & VRING_DESC_F_WRITE) != 0 {
                            if sector_offset + current_offset < self.disk_image.len() {
                                let available =
                                    self.disk_image.len() - (sector_offset + current_offset);
                                let copy_len = std::cmp::min(d_len as usize, available);
                                self.dma_write(
                                    d_addr,
                                    &self.disk_image[sector_offset + current_offset
                                        ..sector_offset + current_offset + copy_len],
                                );
                                len_written += copy_len as u32;
                            }
                        }
                        current_offset += d_len as usize;
                    }
                }

                self.dma_write(s_addr, &[0]);
                if is_write {
                    len_written = 0;
                }
            }

            let used_idx_addr = used_addr + 2;
            let current_used =
                u16::from_le_bytes(self.dma_read(used_idx_addr, 2).try_into().unwrap());
            let used_elem = used_addr + 4 + (current_used as u64 % self.queue_num as u64) * 8;

            self.dma_write(used_elem, &u32::from(head_idx).to_le_bytes());
            self.dma_write(used_elem + 4, &(len_written as u32).to_le_bytes());
            self.dma_write(used_idx_addr, &current_used.wrapping_add(1).to_le_bytes());

            self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
        }
        self.interrupt_status |= 1;
    }
}

impl Device for VirtioBlock {
    /// Returns the device name.
    fn name(&self) -> &str {
        "VirtIO-Blk"
    }

    /// Returns the address range (Base, Size).
    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x1000)
    }

    /// Reads a word (32-bit) from the device.
    ///
    /// Handles reads from VirtIO MMIO registers (Magic, Version, DeviceID, Status, etc.).
    fn read_u32(&mut self, offset: u64) -> u32 {
        match offset {
            REG_MAGIC => VIRTIO_MMIO_MAGIC_VALUE,
            REG_VERSION => VIRTIO_VERSION_VALUE,
            REG_DEVICE_ID => VIRTIO_MMIO_DEVICE_ID_VALUE,
            REG_VENDOR_ID => VIRTIO_MMIO_VENDOR_ID_VALUE,
            REG_DEVICE_FEATURES => {
                if self.device_features_sel == 1 {
                    1
                } else {
                    0
                }
            }
            REG_QUEUE_NUM_MAX => QUEUE_NUM_MAX_VALUE,
            REG_QUEUE_READY => self.queue_ready,
            REG_INTERRUPT_STATUS => self.interrupt_status,
            REG_STATUS => self.status,
            _ => {
                if offset >= REG_CONFIG_BASE && offset < REG_CONFIG_BASE + 0x100 {
                    let config_offset = offset - REG_CONFIG_BASE;
                    match config_offset {
                        0 => (self.disk_image.len() as u64 / SECTOR_SIZE) as u32,
                        4 => ((self.disk_image.len() as u64 / SECTOR_SIZE) >> 32) as u32,
                        _ => 0,
                    }
                } else {
                    0
                }
            }
        }
    }

    /// Writes a word (32-bit) to the device.
    ///
    /// Handles writes to VirtIO MMIO registers (Status, Queue configuration, Notify).
    fn write_u32(&mut self, offset: u64, val: u32) {
        match offset {
            REG_DEVICE_FEATURES_SEL => self.device_features_sel = val,
            REG_DRIVER_FEATURES => {}
            REG_DRIVER_FEATURES_SEL => self.driver_features_sel = val,
            REG_QUEUE_SEL => {}
            REG_QUEUE_NUM => self.queue_num = val,
            REG_QUEUE_READY => self.queue_ready = val,
            REG_QUEUE_NOTIFY => {
                self.queue_notify = val;
                self.process_queue();
            }
            REG_INTERRUPT_ACK => self.interrupt_status &= !val,
            REG_STATUS => self.status = val,
            REG_QUEUE_DESC_LOW => self.queue_desc_low = val,
            REG_QUEUE_DESC_HIGH => self.queue_desc_high = val,
            REG_QUEUE_AVAIL_LOW => self.queue_avail_low = val,
            REG_QUEUE_AVAIL_HIGH => self.queue_avail_high = val,
            REG_QUEUE_USED_LOW => self.queue_used_low = val,
            REG_QUEUE_USED_HIGH => self.queue_used_high = val,
            _ => {}
        }
    }

    /// Reads a byte (delegates to read_u32).
    fn read_u8(&mut self, offset: u64) -> u8 {
        (self.read_u32(offset & !3) >> ((offset & 3) * 8)) as u8
    }
    /// Reads a half-word (delegates to read_u32).
    fn read_u16(&mut self, offset: u64) -> u16 {
        (self.read_u32(offset & !3) >> ((offset & 3) * 8)) as u16
    }
    /// Reads a double-word (delegates to read_u32).
    fn read_u64(&mut self, offset: u64) -> u64 {
        self.read_u32(offset) as u64
    }
    /// Writes a byte (delegates to write_u32).
    fn write_u8(&mut self, offset: u64, val: u8) {
        self.write_u32(offset & !3, val as u32);
    }
    /// Writes a half-word (delegates to write_u32).
    fn write_u16(&mut self, offset: u64, val: u16) {
        self.write_u32(offset & !3, val as u32);
    }
    /// Writes a double-word (delegates to write_u32).
    fn write_u64(&mut self, offset: u64, val: u64) {
        self.write_u32(offset, val as u32);
    }

    /// Advances the device state.
    ///
    /// Returns true if an interrupt is pending.
    fn tick(&mut self) -> bool {
        (self.interrupt_status & 1) != 0
    }

    /// Returns the Interrupt Request (IRQ) ID associated with this device.
    fn get_irq_id(&self) -> Option<u32> {
        Some(1)
    }
}
