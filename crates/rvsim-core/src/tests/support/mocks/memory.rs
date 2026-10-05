//! Test memory mocks updated for the packet-based `Handle` trait.

use crate::common::LineAddr;
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::packet::{HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;

/// MMIO-region mock backed by a byte vector. Implements the new
/// `Handle` + `Device` API so it can be added to a test `Bus`.
pub struct MockMemory {
    data: Vec<u8>,
    base: u64,
}

impl MockMemory {
    pub fn new(size: usize, base: u64) -> Self {
        Self { data: vec![0; size], base }
    }

    fn read_bytes(&self, offset: u64, size: usize) -> u64 {
        let idx = offset as usize;
        let end = idx + size;
        if end > self.data.len() {
            return 0;
        }
        match size {
            1 => u64::from(self.data[idx]),
            2 => u64::from(u16::from_le_bytes(self.data[idx..end].try_into().unwrap())),
            4 => u64::from(u32::from_le_bytes(self.data[idx..end].try_into().unwrap())),
            8 => u64::from_le_bytes(self.data[idx..end].try_into().unwrap()),
            _ => 0,
        }
    }

    fn write_bytes(&mut self, offset: u64, size: usize, val: u64) {
        let idx = offset as usize;
        let end = idx + size;
        if end > self.data.len() {
            return;
        }
        match size {
            1 => self.data[idx] = val as u8,
            2 => self.data[idx..end].copy_from_slice(&(val as u16).to_le_bytes()),
            4 => self.data[idx..end].copy_from_slice(&(val as u32).to_le_bytes()),
            8 => self.data[idx..end].copy_from_slice(&val.to_le_bytes()),
            _ => {}
        }
    }
}

impl Handle for MockMemory {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base);
            let width = size.bytes();
            let value = match op {
                MemOp::Read | MemOp::ReadOwn | MemOp::Fetch | MemOp::Atomic { .. } => {
                    self.read_bytes(offset, width)
                }
                MemOp::Write { data: WriteData::Small(v), .. } => {
                    self.write_bytes(offset, width, v);
                    0
                }
                MemOp::Write { data: WriteData::Line { bytes, mask }, .. } => {
                    let base = offset as usize;
                    for (i, byte) in bytes.iter().enumerate() {
                        if mask >> i & 1 == 1 && base + i < self.data.len() {
                            self.data[base + i] = *byte;
                        }
                    }
                    0
                }
                MemOp::Writeback { .. } | MemOp::Maintain { .. } | MemOp::Prefetch { .. } => 0,
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

impl Device for MockMemory {
    fn name(&self) -> &'static str {
        "MockMemory"
    }
    fn address_range(&self) -> (u64, u64) {
        (self.base, self.data.len() as u64)
    }
}
