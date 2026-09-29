//! Memory Controller Unit Tests.
//!
//! Verifies SimpleController (fixed latency) and DramController
//! (multi-bank, row-buffer-aware, refresh-capable DRAM timing) via the
//! event-driven Handle interface: each test sends a MemReq packet and
//! inspects the scheduled MemResp's fire_at to recover the access latency
//! the controller computed.

use rvsim_core::common::{HartId, PhysAddr};
use rvsim_core::config::Config;
use rvsim_core::sim::components::{ComponentId, MemCtrlId, PipelineId, ReqId};
use rvsim_core::sim::events::EventQueue;
use rvsim_core::sim::handle::{Handle, HandleCtx};
use rvsim_core::sim::memory::GlobalMemory;
use rvsim_core::sim::packet::WriteOrigin;
use rvsim_core::sim::packet::{AccessSize, MemOp, Packet, WriteData};
use rvsim_core::sim::stats::Stats;
use rvsim_core::soc::memory::buffer::DramBuffer;
use rvsim_core::soc::memory::controller::{
    Bandwidth, DramConfig, DramController, SimpleController,
};
use std::num::NonZeroU64;
use std::sync::Arc;

/// Issues a `MemReq::Read` at `paddr` at `cycle` and returns the latency the
/// controller computed (fire_at − cycle of the scheduled `MemResp`).
fn read_latency<H: Handle>(ctrl: &mut H, paddr: u64, cycle: u64) -> u64 {
    let req_id = ReqId::new(u64::MAX);
    let mut queue = EventQueue::new();
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut ctx = HandleCtx {
        scheduler: &mut queue,
        stats: &mut stats,
        memory: &mut memory,
        config: &config,
        cycle,
        self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
    };
    ctrl.handle(
        Packet::MemReq {
            req_id,
            paddr: PhysAddr::new(paddr),
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Read,
        },
        ComponentId::Pipeline(PipelineId::new(0)),
        &mut ctx,
    );
    while let Some(event) = queue.pop_ready(u64::MAX) {
        if let Packet::MemResp { req_id: rid, .. } = event.packet
            && rid == req_id
        {
            return event.fire_at - cycle;
        }
    }
    panic!("no MemResp scheduled");
}

fn write_latency<H: Handle>(ctrl: &mut H, paddr: u64, cycle: u64) -> u64 {
    let req_id = ReqId::new(u64::MAX);
    let mut queue = EventQueue::new();
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut ctx = HandleCtx {
        scheduler: &mut queue,
        stats: &mut stats,
        memory: &mut memory,
        config: &config,
        cycle,
        self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
    };
    ctrl.handle(
        Packet::MemReq {
            req_id,
            paddr: PhysAddr::new(paddr),
            vaddr: None,
            size: AccessSize::B8,
            op: MemOp::Write {
                data: WriteData::Small(0),
                origin: WriteOrigin::Hart(HartId::new(0)),
            },
        },
        ComponentId::Pipeline(PipelineId::new(0)),
        &mut ctx,
    );
    while let Some(event) = queue.pop_ready(u64::MAX) {
        if let Packet::MemResp { req_id: rid, .. } = event.packet
            && rid == req_id
        {
            return event.fire_at - cycle;
        }
    }
    panic!("no MemResp scheduled");
}

/// A simple controller moving `bytes_per_cycle` at a 1 Hz core clock.
fn simple_with_bandwidth(latency: u64, bytes_per_cycle: u64) -> SimpleController {
    let buffer = Arc::new(DramBuffer::new(0x10000));
    let bandwidth = Bandwidth::new(NonZeroU64::new(bytes_per_cycle).unwrap(), 1);
    SimpleController::new(buffer, PhysAddr::new(0), latency, bandwidth)
}

/// A simple controller whose bandwidth never holds a request up.
fn simple(latency: u64) -> SimpleController {
    simple_with_bandwidth(latency, 64)
}

/// The latency of a line read issued at `cycle`.
fn line_read_latency(ctrl: &mut SimpleController, paddr: u64, cycle: u64) -> u64 {
    let req_id = ReqId::new(u64::MAX);
    let mut queue = EventQueue::new();
    let mut stats = Stats::new();
    let mut memory = GlobalMemory::new(None, 1, 64);
    let config = Config::default();
    let mut ctx = HandleCtx {
        scheduler: &mut queue,
        stats: &mut stats,
        memory: &mut memory,
        config: &config,
        cycle,
        self_id: ComponentId::MemCtrl(MemCtrlId::new(0)),
    };
    ctrl.handle(
        Packet::MemReq {
            req_id,
            paddr: PhysAddr::new(paddr),
            vaddr: None,
            size: AccessSize::Line,
            op: MemOp::Read,
        },
        ComponentId::Pipeline(PipelineId::new(0)),
        &mut ctx,
    );
    while let Some(event) = queue.pop_ready(u64::MAX) {
        if let Packet::MemResp { req_id: rid, .. } = event.packet
            && rid == req_id
        {
            return event.fire_at - cycle;
        }
    }
    panic!("no MemResp scheduled");
}

fn dram_no_refresh(t_cas: u64, t_ras: u64, t_pre: u64) -> DramController {
    let buffer = Arc::new(DramBuffer::new(0x10000));
    DramController::new(
        buffer,
        PhysAddr::new(0),
        DramConfig {
            t_cas,
            t_ras,
            t_pre,
            t_rrd: 4,
            num_banks: 8,
            row_size_bytes: 2048,
            t_refi: 0,
            t_rfc: 0,
        },
    )
}

fn dram_default() -> DramController {
    dram_no_refresh(5, 10, 8)
}

/// Returns an address in the given bank and row-group.
/// bank 0 row 0 = 0x0000, bank 0 row 1 = 0x4000, bank 1 row 0 = 0x0800, etc.
fn addr(bank: usize, row_group: u64) -> u64 {
    (row_group * 8 + bank as u64) * 2048
}

#[test]
fn simple_controller_fixed_latency() {
    let mut ctrl = simple(10);
    assert_eq!(read_latency(&mut ctrl, 0x1000, 0), 10);
    assert_eq!(read_latency(&mut ctrl, 0x2000, 100), 10);
    assert_eq!(read_latency(&mut ctrl, 0x3000, 200), 10);
}

#[test]
fn simple_controller_zero_latency() {
    let mut ctrl = simple(0);
    assert_eq!(read_latency(&mut ctrl, 0, 0), 0);
}

#[test]
fn simple_controller_write_same_as_read() {
    let mut ctrl = simple(15);
    assert_eq!(write_latency(&mut ctrl, 0x1000, 0), 15);
}

#[test]
fn simple_controller_requests_wait_for_the_bandwidth() {
    let mut ctrl = simple_with_bandwidth(10, 8);

    assert_eq!(line_read_latency(&mut ctrl, 0x1000, 0), 10, "an idle controller answers at once");
    assert_eq!(line_read_latency(&mut ctrl, 0x2000, 0), 18, "the second line waits 64 / 8 cycles");
    assert_eq!(line_read_latency(&mut ctrl, 0x3000, 40), 10, "the controller is free again");
}

#[test]
fn dram_first_access_full_latency() {
    let mut ctrl = dram_default();
    // Cold start: t_ras (activate) + t_cas (column access) for the first read.
    let lat = read_latency(&mut ctrl, addr(0, 0), 0);
    assert_eq!(lat, 10 + 5);
}

#[test]
fn dram_row_buffer_hit() {
    let mut ctrl = dram_default();
    // First access opens the row.
    let _ = read_latency(&mut ctrl, addr(0, 0), 0);
    // Second access to same row (different address in same 2KB row): t_cas only.
    let lat = read_latency(&mut ctrl, addr(0, 0) + 64, 100);
    assert_eq!(lat, 5);
}

#[test]
fn dram_row_buffer_conflict_same_bank() {
    let mut ctrl = dram_default();
    // Open row 0 in bank 0.
    let _ = read_latency(&mut ctrl, addr(0, 0), 0);
    // Switch to row 1 in bank 0: t_pre (precharge) + t_ras + t_cas.
    let lat = read_latency(&mut ctrl, addr(0, 1), 100);
    assert_eq!(lat, 8 + 10 + 5);
}

#[test]
fn dram_independent_banks_no_conflict() {
    let mut ctrl = dram_default();
    let _ = read_latency(&mut ctrl, addr(0, 0), 0);
    // Different bank: full activate again (cold), no precharge cost.
    let lat = read_latency(&mut ctrl, addr(1, 0), 100);
    assert_eq!(lat, 10 + 5);
}
