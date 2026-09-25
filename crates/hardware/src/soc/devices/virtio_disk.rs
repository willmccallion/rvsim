//! VirtIO Block Device (MMIO).
//!
//! Implements a `VirtIO` block device over Memory-Mapped I/O (MMIO) for disk access.
//! Supports the legacy `VirtIO` interface required by the Linux kernel.

use crate::common::{IrqId, LineAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, Packet, WriteData, MesiState};
use crate::soc::devices::Device;
use crate::common::PhysAddr;
use crate::soc::memory::buffer::DramBuffer;
use std::sync::Arc;

/// `VirtIO` MMIO magic value register offset.
const REG_MAGIC: u64 = 0x00;

/// `VirtIO` MMIO version register offset.
const REG_VERSION: u64 = 0x04;

/// `VirtIO` MMIO device ID register offset.
const REG_DEVICE_ID: u64 = 0x08;

/// `VirtIO` MMIO vendor ID register offset.
const REG_VENDOR_ID: u64 = 0x0c;

/// `VirtIO` MMIO device features register offset.
const REG_DEVICE_FEATURES: u64 = 0x10;

/// `VirtIO` MMIO device features select register offset.
const REG_DEVICE_FEATURES_SEL: u64 = 0x14;

/// `VirtIO` MMIO driver features register offset — writes ignored (no feature negotiation).
const _REG_DRIVER_FEATURES: u64 = 0x20;

/// `VirtIO` MMIO driver features select register offset.
const REG_DRIVER_FEATURES_SEL: u64 = 0x24;

/// `VirtIO` MMIO queue select register offset — writes ignored.
const _REG_QUEUE_SEL: u64 = 0x30;

/// `VirtIO` MMIO queue maximum size register offset.
const REG_QUEUE_NUM_MAX: u64 = 0x34;

/// `VirtIO` MMIO queue size register offset.
const REG_QUEUE_NUM: u64 = 0x38;

/// `VirtIO` MMIO queue ready register offset.
const REG_QUEUE_READY: u64 = 0x44;

/// `VirtIO` MMIO queue notify register offset.
const REG_QUEUE_NOTIFY: u64 = 0x50;

/// `VirtIO` MMIO interrupt status register offset.
const REG_INTERRUPT_STATUS: u64 = 0x60;

/// `VirtIO` MMIO interrupt acknowledge register offset.
const REG_INTERRUPT_ACK: u64 = 0x64;

/// `VirtIO` MMIO device status register offset.
const REG_STATUS: u64 = 0x70;

/// `VirtIO` MMIO queue descriptor table address (low 32 bits) register offset.
const REG_QUEUE_DESC_LOW: u64 = 0x80;

/// `VirtIO` MMIO queue descriptor table address (high 32 bits) register offset.
const REG_QUEUE_DESC_HIGH: u64 = 0x84;

/// `VirtIO` MMIO queue available ring address (low 32 bits) register offset.
const REG_QUEUE_AVAIL_LOW: u64 = 0x90;

/// `VirtIO` MMIO queue available ring address (high 32 bits) register offset.
const REG_QUEUE_AVAIL_HIGH: u64 = 0x94;

/// `VirtIO` MMIO queue used ring address (low 32 bits) register offset.
const REG_QUEUE_USED_LOW: u64 = 0xa0;

/// `VirtIO` MMIO queue used ring address (high 32 bits) register offset.
const REG_QUEUE_USED_HIGH: u64 = 0xa4;

/// `VirtIO` MMIO configuration space base offset.
const REG_CONFIG_BASE: u64 = 0x100;

/// `VirtIO` MMIO magic value ("virt" in ASCII: 0x74726976).
const VIRTIO_MMIO_MAGIC_VALUE: u32 = 0x74726976;

/// `VirtIO` MMIO vendor ID value (QEMU vendor: 0x554d4551).
const VIRTIO_MMIO_VENDOR_ID_VALUE: u32 = 0x554d4551;

/// `VirtIO` MMIO device ID for block device (2).
const VIRTIO_MMIO_DEVICE_ID_VALUE: u32 = 2;

/// `VirtIO` specification version (2).
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

/// `VirtIO` Block device structure.
///
/// Implements a memory-mapped block device compliant with the `VirtIO` specification.
/// It uses a shared DRAM buffer to perform DMA operations for reading and writing
/// disk sectors.
#[derive(Debug)]
pub struct VirtioBlock {
    /// Base physical address of the device MMIO region.
    base_addr: u64,
    /// Base physical address of system RAM.
    ram_base: u64,
    /// Disk image data.
    disk_image: Vec<u8>,
    /// Shared reference to system RAM for DMA.
    ram: Arc<DramBuffer>,
    /// DMA writes not yet published to the system.
    dma_writes: Vec<(PhysAddr, usize)>,

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
    /// Creates a new `VirtIO` Block device.
    ///
    /// # Arguments
    ///
    /// * `base_addr` - MMIO base address.
    /// * `ram_base` - System RAM base address.
    /// * `ram` - Shared DRAM buffer for DMA access.
    pub const fn new(base_addr: u64, ram_base: u64, ram: Arc<DramBuffer>) -> Self {
        Self {
            base_addr,
            ram_base,
            disk_image: Vec::new(),
            ram,
            dma_writes: Vec::new(),
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
    pub fn load(&mut self, data: Vec<u8>) {
        self.disk_image = data;
    }

    /// Reads `len` bytes from system RAM at physical address `addr` via DMA.
    /// Returns zeroed bytes if the address is out of bounds.
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

    fn dma_read_u16(&self, addr: u64) -> u16 {
        let b = self.dma_read(addr, 2);
        u16::from_le_bytes([b[0], b[1]])
    }

    fn dma_read_u32(&self, addr: u64) -> u32 {
        let b = self.dma_read(addr, 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    fn dma_read_u64(&self, addr: u64) -> u64 {
        let b = self.dma_read(addr, 8);
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }

    /// Writes `data` to system RAM at physical address `addr` via DMA.
    fn dma_write(&mut self, addr: u64, data: &[u8]) {
        if addr < self.ram_base {
            println!("[VirtIO] DMA Write Out of Bounds (Low): 0x{addr:x}");
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
        self.dma_writes.push((PhysAddr::new(addr), data.len()));
    }

    /// Processes the `VirtQueue` (triggered on Queue Notify write).
    fn process_queue(&mut self) {
        if self.queue_num == 0 {
            return;
        }

        let desc_addr = ((self.queue_desc_high as u64) << 32) | (self.queue_desc_low as u64);
        let avail_addr = ((self.queue_avail_high as u64) << 32) | (self.queue_avail_low as u64);
        let used_addr = ((self.queue_used_high as u64) << 32) | (self.queue_used_low as u64);

        let avail_idx = self.dma_read_u16(avail_addr + 2);

        while self.last_avail_idx != avail_idx {
            let ring_offset = 4 + (self.last_avail_idx as u64 % self.queue_num as u64) * 2;
            let head_idx = self.dma_read_u16(avail_addr + ring_offset);

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
                let addr = self.dma_read_u64(addr_offset + DESC_OFFSET_ADDR);
                let len = self.dma_read_u32(addr_offset + DESC_OFFSET_LEN);
                let flags = self.dma_read_u16(addr_offset + DESC_OFFSET_FLAGS);
                let next = self.dma_read_u16(addr_offset + DESC_OFFSET_NEXT);

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
                let type_val = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
                let sector = u64::from_le_bytes([
                    header[8], header[9], header[10], header[11], header[12], header[13],
                    header[14], header[15],
                ]);
                let is_write = type_val == 1;
                let is_flush = type_val == 4;

                let (s_addr, _, _) = descriptors[descriptors.len() - 1];

                let sector_offset = (sector * SECTOR_SIZE) as usize;
                let mut current_offset = 0;

                if is_flush {
                    // no-op: flush has no data transfer
                } else if is_write {
                    let mut current_disk_offset = sector_offset;

                    for (d_addr, d_len, _) in &descriptors[1..descriptors.len() - 1] {
                        let data = self.dma_read(*d_addr, *d_len as usize);
                        if current_disk_offset + data.len() <= self.disk_image.len() {
                            self.disk_image[current_disk_offset..current_disk_offset + data.len()]
                                .copy_from_slice(&data);
                        }
                        current_disk_offset += *d_len as usize;
                        len_written += *d_len;
                    }
                } else {
                    for (d_addr, d_len, d_flags) in &descriptors[1..descriptors.len() - 1] {
                        if (d_flags & VRING_DESC_F_WRITE) != 0
                            && sector_offset + current_offset < self.disk_image.len()
                        {
                            let available =
                                self.disk_image.len() - (sector_offset + current_offset);
                            let copy_len = std::cmp::min(*d_len as usize, available);
                            let start = sector_offset + current_offset;
                            let sector_data = self.disk_image[start..start + copy_len].to_vec();
                            self.dma_write(*d_addr, &sector_data);
                            len_written += copy_len as u32;
                        }
                        current_offset += *d_len as usize;
                    }
                }

                self.dma_write(s_addr, &[0]);
            }

            let used_idx_addr = used_addr + 2;
            let current_used = self.dma_read_u16(used_idx_addr);
            let used_elem = used_addr + 4 + (current_used as u64 % self.queue_num as u64) * 8;

            self.dma_write(used_elem, &u32::from(head_idx).to_le_bytes());
            self.dma_write(used_elem + 4, &len_written.to_le_bytes());
            self.dma_write(used_idx_addr, &current_used.wrapping_add(1).to_le_bytes());

            self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
        }
        self.interrupt_status |= 1;
    }
}

impl VirtioBlock {
    fn read_u32_reg(&self, offset: u64) -> u32 {
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
                if (REG_CONFIG_BASE..REG_CONFIG_BASE + 0x100).contains(&offset) {
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

    fn write_u32_reg(&mut self, offset: u64, val: u32) {
        match offset {
            REG_DEVICE_FEATURES_SEL => self.device_features_sel = val,
            REG_DRIVER_FEATURES_SEL => self.driver_features_sel = val,
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
}

impl Handle for VirtioBlock {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let value: u64 = match (size, op) {
                (AccessSize::B4 | AccessSize::B8, MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. }) => {
                    u64::from(self.read_u32_reg(offset))
                }
                (AccessSize::B1, MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. }) => {
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    u64::from((self.read_u32_reg(aligned) >> shift) as u8)
                }
                (AccessSize::B2, MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. }) => {
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    u64::from((self.read_u32_reg(aligned) >> shift) as u16)
                }
                (AccessSize::B4 | AccessSize::B8, MemOp::Write { data: WriteData::Small(val) }) => {
                    self.write_u32_reg(offset, val as u32);
                    0
                }
                (AccessSize::B1 | AccessSize::B2, MemOp::Write { data: WriteData::Small(val) }) => {
                    self.write_u32_reg(offset & !3, val as u32);
                    0
                }
                _ => 0,
            };
            ctx.scheduler.schedule(
                ctx.cycle + 1,
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, 64),
                    data: MemRespData::Small(value),
                    hit_level: HitLevel::Mmio,
                state: MesiState::Exclusive,
                },
            );
        }
    }
}

impl Device for VirtioBlock {
    fn take_dma_writes(&mut self) -> Vec<(PhysAddr, usize)> {
        std::mem::take(&mut self.dma_writes)
    }

    fn name(&self) -> &'static str {
        "VirtIO-Blk"
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 0x1000)
    }

    fn tick(&mut self) -> bool {
        (self.interrupt_status & 1) != 0
    }

    fn get_irq_id(&self) -> Option<IrqId> {
        Some(IrqId::new(1))
    }
}
