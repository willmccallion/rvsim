//! A request's DMA moves over the bus phase by phase.
//!
//! The ring and descriptor reads come first, then the data in line-sized
//! transfers, then the status and used-ring writes. The request completes,
//! and its interrupt is raised, only when the last transfer has returned.

use rvsim_core::common::{LineAddr, PhysAddr};
use rvsim_core::config::Config;
use rvsim_core::sim::components::{ComponentId, DeviceId, PipelineId, ReqId};
use rvsim_core::sim::events::EventQueue;
use rvsim_core::sim::handle::{Handle, HandleCtx};
use rvsim_core::sim::packet::WriteOrigin;
use rvsim_core::sim::packet::{
    AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData,
};
use rvsim_core::sim::state::global_memory::GlobalMemory;
use rvsim_core::sim::stats::Stats;
use rvsim_core::soc::devices::Device;
use rvsim_core::soc::devices::virtio_disk::VirtioBlock;
use rvsim_core::soc::memory::buffer::DramBuffer;
use std::sync::Arc;

pub(super) const MMIO: u64 = 0x1000_1000;
pub(super) const RAM_BASE: u64 = 0x8000_0000;
const DESC: u64 = 0x1000;
const AVAIL: u64 = 0x2000;
const USED: u64 = 0x3000;
const HEADER: u64 = 0x4000;
pub(super) const DATA: u64 = 0x5000;
const STATUS: u64 = 0x6000;
pub(super) const SECTOR: u64 = 1;
const NEXT: u16 = 1;
const WRITE: u16 = 2;

fn descriptor(ram: &DramBuffer, index: u64, addr: u64, len: u32, flags: u16, next: u16) {
    let mut bytes = Vec::new();
    bytes.extend((RAM_BASE + addr).to_le_bytes());
    bytes.extend(len.to_le_bytes());
    bytes.extend(flags.to_le_bytes());
    bytes.extend(next.to_le_bytes());
    ram.write_slice((DESC + index * 16) as usize, &bytes);
}

/// The request a test queues.
#[derive(Clone, Copy)]
pub(super) enum Request {
    /// Reads `SECTOR` into `DATA`.
    Read,
    /// Writes `DATA` to `SECTOR`.
    Write,
}

/// A device with a 2 KiB disk and one queued read of `SECTOR` into `DATA`.
fn device_with_a_queued_read() -> (VirtioBlock, Arc<DramBuffer>) {
    device_with_a_queued(Request::Read)
}

/// The 2 KiB disk image the test devices load.
pub(super) fn disk_image() -> Vec<u8> {
    (0..2048u32).map(|i| (i % 251) as u8).collect()
}

/// A device with `disk_image` loaded and one `request` queued.
pub(super) fn device_with_a_queued(request: Request) -> (VirtioBlock, Arc<DramBuffer>) {
    let ram = Arc::new(DramBuffer::new(0x10000));
    let mut device = VirtioBlock::new(MMIO, RAM_BASE, Arc::clone(&ram));
    device.load(disk_image());

    let (request_type, data_flags) = match request {
        Request::Read => (0u32, NEXT | WRITE),
        Request::Write => (1u32, NEXT),
    };
    descriptor(&ram, 0, HEADER, 16, NEXT, 1);
    descriptor(&ram, 1, DATA, 512, data_flags, 2);
    descriptor(&ram, 2, STATUS, 1, WRITE, 0);
    let mut header = vec![0u8; 16];
    header[0..4].copy_from_slice(&request_type.to_le_bytes());
    header[8..16].copy_from_slice(&SECTOR.to_le_bytes());
    ram.write_slice(HEADER as usize, &header);
    ram.write_slice(AVAIL as usize, &[0, 0, 1, 0, 0, 0]);

    for (register, value) in [
        (0x80, RAM_BASE + DESC),
        (0x84, 0),
        (0x90, RAM_BASE + AVAIL),
        (0x94, 0),
        (0xa0, RAM_BASE + USED),
        (0xa4, 0),
        (0x38, 8),
        (0x44, 1),
    ] {
        crate::common::probe::write(&mut device, PhysAddr::new(MMIO + register), value, 4);
    }
    (device, ram)
}

fn deliver(
    device: &mut VirtioBlock,
    queue: &mut EventQueue,
    cycle: u64,
    source: ComponentId,
    packet: Packet,
) {
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut ctx = HandleCtx {
        scheduler: queue,
        stats: &mut stats,
        memory: &mut memory,
        config: &config,
        cycle,
        self_id: ComponentId::Device(DeviceId::new(0)),
    };
    device.handle(packet, source, &mut ctx);
}

/// Rings the device's doorbell, starting its queued request.
pub(super) fn notify(device: &mut VirtioBlock, queue: &mut EventQueue) {
    let doorbell = Packet::MemReq {
        req_id: ReqId::new(1),
        paddr: PhysAddr::new(MMIO + 0x50),
        vaddr: None,
        size: AccessSize::B4,
        op: MemOp::Write { data: WriteData::Small(0), origin: WriteOrigin::Placed },
    };
    deliver(device, queue, 0, ComponentId::Pipeline(PipelineId::new(0)), doorbell);
}

/// Takes the DMA requests the device has put on the bus.
fn dma_requests(queue: &mut EventQueue) -> Vec<(ReqId, PhysAddr, AccessSize, bool)> {
    let mut requests = Vec::new();
    while let Some(event) = queue.pop_ready(u64::MAX) {
        if let (ComponentId::Bus, Packet::MemReq { req_id, paddr, size, op, .. }) =
            (event.target, event.packet)
        {
            requests.push((req_id, paddr, size, matches!(op, MemOp::Write { .. })));
        }
    }
    requests
}

fn answer(
    device: &mut VirtioBlock,
    queue: &mut EventQueue,
    cycle: u64,
    requests: &[(ReqId, PhysAddr, AccessSize, bool)],
) {
    for (req_id, paddr, _, _) in requests {
        let response = Packet::MemResp {
            req_id: *req_id,
            line_addr: LineAddr::from_phys(*paddr, 64),
            data: MemRespData::Small(0),
            hit_level: HitLevel::Dram,
            state: MesiState::Exclusive,
        };
        deliver(device, queue, cycle, ComponentId::Bus, response);
    }
}

fn used_idx(ram: &DramBuffer) -> u16 {
    let bytes = ram.read_slice((USED + 2) as usize, 2);
    u16::from_le_bytes([bytes[0], bytes[1]])
}

#[test]
fn a_read_request_completes_after_its_three_dma_phases() {
    let (mut device, ram) = device_with_a_queued_read();
    let mut queue = EventQueue::new();

    notify(&mut device, &mut queue);

    let control = dma_requests(&mut queue);
    assert_eq!(control.len(), 10, "avail index and entry, three descriptors, the header");
    assert!(control.iter().all(|(_, _, _, write)| !write), "the control phase only reads");
    assert!(!device.tick(), "no interrupt before the data moved");
    answer(&mut device, &mut queue, 10, &control);

    let data = dma_requests(&mut queue);
    assert_eq!(data.len(), 8, "512 bytes are eight line transfers");
    assert!(data.iter().all(|(_, _, size, write)| *size == AccessSize::Line && *write));
    assert_eq!(
        ram.read_slice(DATA as usize, 4),
        &[0, 0, 0, 0],
        "the data lands when the request completes"
    );
    answer(&mut device, &mut queue, 20, &data);

    let completion = dma_requests(&mut queue);
    let sizes: Vec<AccessSize> = completion.iter().map(|(_, _, size, _)| *size).collect();
    assert_eq!(
        sizes,
        [AccessSize::B1, AccessSize::B8, AccessSize::B2],
        "status, used entry, used index"
    );
    assert!(!device.tick(), "no interrupt before the used ring is written");
    assert_eq!(used_idx(&ram), 0);
    answer(&mut device, &mut queue, 30, &completion);

    let expected: Vec<u8> = (512..1024u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(
        ram.read_slice(DATA as usize, 512),
        &expected[..],
        "sector 1 was read into the buffer"
    );
    assert_eq!(used_idx(&ram), 1);
    assert!(device.tick(), "the interrupt is raised once the request completes");
    assert!(dma_requests(&mut queue).is_empty(), "nothing else was queued");
}

#[test]
fn draining_the_device_completes_a_queued_request_at_once() {
    let (mut device, ram) = device_with_a_queued_read();
    let mut queue = EventQueue::new();
    notify(&mut device, &mut queue);
    assert_eq!(dma_requests(&mut queue).len(), 10, "the request is in flight");

    device.drain();

    let expected: Vec<u8> = (512..1024u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(ram.read_slice(DATA as usize, 512), &expected[..], "the data landed");
    assert_eq!(used_idx(&ram), 1);
    assert!(device.tick(), "the interrupt is raised");
    assert!(!device.take_dma_writes().is_empty(), "the writes are reported for publishing");
}
