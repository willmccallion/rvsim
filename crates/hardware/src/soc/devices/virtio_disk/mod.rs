//! The virtio block device (MMIO).
//!
//! Implements a virtio block device over Memory-Mapped I/O (MMIO) for disk access.
//! Supports the legacy virtio interface required by the Linux kernel.

mod checkpoint;
mod queue;

use crate::common::{IrqId, LineAddr, PhysAddr};
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;
use crate::soc::memory::buffer::DramBuffer;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// virtio MMIO magic value register offset.
const REG_MAGIC: u64 = 0x00;

/// virtio MMIO version register offset.
const REG_VERSION: u64 = 0x04;

/// virtio MMIO device ID register offset.
const REG_DEVICE_ID: u64 = 0x08;

/// virtio MMIO vendor ID register offset.
const REG_VENDOR_ID: u64 = 0x0c;

/// virtio MMIO device features register offset.
const REG_DEVICE_FEATURES: u64 = 0x10;

/// virtio MMIO device features select register offset.
const REG_DEVICE_FEATURES_SEL: u64 = 0x14;

/// virtio MMIO driver features register offset — writes ignored (no feature negotiation).
const _REG_DRIVER_FEATURES: u64 = 0x20;

/// virtio MMIO driver features select register offset.
const REG_DRIVER_FEATURES_SEL: u64 = 0x24;

/// virtio MMIO queue select register offset — writes ignored.
const _REG_QUEUE_SEL: u64 = 0x30;

/// virtio MMIO queue maximum size register offset.
const REG_QUEUE_NUM_MAX: u64 = 0x34;

/// virtio MMIO queue size register offset.
const REG_QUEUE_NUM: u64 = 0x38;

/// virtio MMIO queue ready register offset.
const REG_QUEUE_READY: u64 = 0x44;

/// virtio MMIO queue notify register offset.
const REG_QUEUE_NOTIFY: u64 = 0x50;

/// virtio MMIO interrupt status register offset.
const REG_INTERRUPT_STATUS: u64 = 0x60;

/// virtio MMIO interrupt acknowledge register offset.
const REG_INTERRUPT_ACK: u64 = 0x64;

/// virtio MMIO device status register offset.
const REG_STATUS: u64 = 0x70;

/// virtio MMIO queue descriptor table address (low 32 bits) register offset.
const REG_QUEUE_DESC_LOW: u64 = 0x80;

/// virtio MMIO queue descriptor table address (high 32 bits) register offset.
const REG_QUEUE_DESC_HIGH: u64 = 0x84;

/// virtio MMIO queue available ring address (low 32 bits) register offset.
const REG_QUEUE_AVAIL_LOW: u64 = 0x90;

/// virtio MMIO queue available ring address (high 32 bits) register offset.
const REG_QUEUE_AVAIL_HIGH: u64 = 0x94;

/// virtio MMIO queue used ring address (low 32 bits) register offset.
const REG_QUEUE_USED_LOW: u64 = 0xa0;

/// virtio MMIO queue used ring address (high 32 bits) register offset.
const REG_QUEUE_USED_HIGH: u64 = 0xa4;

/// virtio MMIO configuration space base offset.
const REG_CONFIG_BASE: u64 = 0x100;

/// virtio MMIO magic value ("virt" in ASCII: 0x74726976).
const VIRTIO_MMIO_MAGIC_VALUE: u32 = 0x74726976;

/// virtio MMIO vendor ID value (QEMU vendor: 0x554d4551).
const VIRTIO_MMIO_VENDOR_ID_VALUE: u32 = 0x554d4551;

/// virtio MMIO device ID for block device (2).
const VIRTIO_MMIO_DEVICE_ID_VALUE: u32 = 2;

/// virtio specification version (2).
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

/// Bytes a DMA transfer moves at most: one cache line.
const LINE_BYTES: u64 = 64;

/// virtio Block device structure.
///
/// Implements a memory-mapped block device compliant with the virtio specification.
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
    /// Digest of the image as loaded.
    image_digest: u64,
    /// Sectors the guest has written since the image was loaded.
    written: BTreeSet<u64>,
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

    /// The request whose DMA is in flight.
    job: Option<DmaJob>,
    /// Sequence number of the next DMA request id.
    next_dma_seq: u64,
}

/// The device's registers, as a checkpoint carries them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtioBlockState {
    /// Device status register.
    pub status: u32,
    /// Configured queue size.
    pub queue_num: u32,
    /// Queue ready bit.
    pub queue_ready: u32,
    /// Queue notify register.
    pub queue_notify: u32,
    /// Descriptor table address, low half.
    pub queue_desc_low: u32,
    /// Descriptor table address, high half.
    pub queue_desc_high: u32,
    /// Available ring address, low half.
    pub queue_avail_low: u32,
    /// Available ring address, high half.
    pub queue_avail_high: u32,
    /// Used ring address, low half.
    pub queue_used_low: u32,
    /// Used ring address, high half.
    pub queue_used_high: u32,
    /// Interrupt status register.
    pub interrupt_status: u32,
    /// Next available ring index to process.
    pub last_avail_idx: u16,
    /// Device features selector.
    pub device_features_sel: u32,
    /// Driver features selector.
    pub driver_features_sel: u32,
    /// Sequence number of the next DMA request id.
    pub next_dma_seq: u64,
    /// Digest of the disk image as loaded, which a restore must match.
    pub image_digest: u64,
    /// Every sector the guest has written since the image was loaded.
    pub written: Vec<WrittenSector>,
}

/// A sector the guest wrote, as a checkpoint carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrittenSector {
    /// The sector number.
    pub sector: u64,
    /// Its contents, as hex.
    pub data: String,
}

/// A 64-bit FNV-1a digest of `bytes`, taken 8 bytes at a time.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn digest(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    let mut chunks = bytes.chunks_exact(8);
    for chunk in &mut chunks {
        let mut word = [0; 8];
        word.copy_from_slice(chunk);
        hash = (hash ^ u64::from_le_bytes(word)).wrapping_mul(FNV_PRIME);
    }
    for &byte in chunks.remainder() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
    }
    hash ^ bytes.len() as u64
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
    text
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| text.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
        .collect()
}

/// One DMA transfer of a request, as the bus sees it.
#[derive(Clone, Copy, Debug)]
struct DmaAccess {
    paddr: PhysAddr,
    size: AccessSize,
    write: bool,
}

impl DmaAccess {
    const fn read(paddr: u64, size: AccessSize) -> Self {
        Self { paddr: PhysAddr::new(paddr), size, write: false }
    }

    const fn write(paddr: u64, size: AccessSize) -> Self {
        Self { paddr: PhysAddr::new(paddr), size, write: true }
    }
}

/// A request in flight: the DMA phases still to issue and the transfers
/// of the current phase not yet answered.
#[derive(Debug)]
struct DmaJob {
    head_idx: u16,
    phases: VecDeque<Vec<DmaAccess>>,
    outstanding: Vec<ReqId>,
}

/// `[addr, addr + len)` as line-sized transfers, one per cache line touched.
fn line_chunks(addr: u64, len: u64, write: bool) -> Vec<DmaAccess> {
    let mut chunks = Vec::new();
    let mut start = addr;
    let end = addr.saturating_add(len);
    while start < end {
        chunks.push(DmaAccess { paddr: PhysAddr::new(start), size: AccessSize::Line, write });
        start = (start | (LINE_BYTES - 1)) + 1;
    }
    chunks
}

unsafe impl Send for VirtioBlock {}
unsafe impl Sync for VirtioBlock {}

impl VirtioBlock {
    /// Creates a new virtio Block device.
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
            image_digest: FNV_OFFSET,
            written: BTreeSet::new(),
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
            job: None,
            next_dma_seq: 0,
        }
    }

    /// Loads a disk image into the device.
    pub fn load(&mut self, data: Vec<u8>) {
        self.image_digest = digest(&data);
        self.written.clear();
        self.disk_image = data;
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

    const fn write_u32_reg(&mut self, offset: u64, val: u32) {
        match offset {
            REG_DEVICE_FEATURES_SEL => self.device_features_sel = val,
            REG_DRIVER_FEATURES_SEL => self.driver_features_sel = val,
            REG_QUEUE_NUM => self.queue_num = val,
            REG_QUEUE_READY => self.queue_ready = val,
            REG_QUEUE_NOTIFY => self.queue_notify = val,
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
        if let Packet::MemResp { req_id, .. } = packet {
            self.on_dma_response(req_id, ctx);
            return;
        }
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let notified = matches!(op, MemOp::Write { .. }) && (offset & !3) == REG_QUEUE_NOTIFY;
            let value: u64 = match (size, op) {
                (
                    AccessSize::B4 | AccessSize::B8,
                    MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. },
                ) => u64::from(self.read_u32_reg(offset)),
                (
                    AccessSize::B1,
                    MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. },
                ) => {
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    u64::from((self.read_u32_reg(aligned) >> shift) as u8)
                }
                (
                    AccessSize::B2,
                    MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. },
                ) => {
                    let aligned = offset & !3;
                    let shift = (offset & 3) * 8;
                    u64::from((self.read_u32_reg(aligned) >> shift) as u16)
                }
                (
                    AccessSize::B4 | AccessSize::B8,
                    MemOp::Write { data: WriteData::Small(val), .. },
                ) => {
                    self.write_u32_reg(offset, val as u32);
                    0
                }
                (
                    AccessSize::B1 | AccessSize::B2,
                    MemOp::Write { data: WriteData::Small(val), .. },
                ) => {
                    self.write_u32_reg(offset & !3, val as u32);
                    0
                }
                _ => 0,
            };
            ctx.scheduler.schedule(
                ctx.cycle + ctx.config.system.device_access_cycles(self.name()),
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
            if notified {
                tracing::trace!(target: "rvsim::dma", cycle = ctx.cycle, busy = self.job.is_some(), "virtio: notified");
                self.start_next_request(ctx);
            }
        }
    }
}

impl Device for VirtioBlock {
    /// A request in flight moves on DMA responses; without one the device
    /// waits for the driver.
    fn quiet_ticks(&self) -> Option<u64> {
        if self.job.is_some() { Some(0) } else { None }
    }

    fn take_dma_writes(&mut self) -> Vec<(PhysAddr, usize)> {
        std::mem::take(&mut self.dma_writes)
    }

    /// Completes the request in flight and every chain still available
    /// at once: after a restore nothing would notify the device again.
    fn drain(&mut self) {
        if let Some(job) = self.job.take() {
            self.complete_request(job.head_idx);
        }
        while let Some((head_idx, _)) = self.next_available_chain() {
            self.complete_request(head_idx);
        }
    }

    fn checkpoint(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.state()).ok()
    }

    fn check_restore(&self, state: &serde_json::Value) -> Result<(), String> {
        let state = serde_json::from_value::<VirtioBlockState>(state.clone())
            .map_err(|error| format!("virtio disk state: {error}"))?;
        self.check_state(&state)
    }

    fn restore(&mut self, state: &serde_json::Value) -> Result<(), String> {
        let state = serde_json::from_value::<VirtioBlockState>(state.clone())
            .map_err(|error| format!("virtio disk state: {error}"))?;
        self.set_state(&state)
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
