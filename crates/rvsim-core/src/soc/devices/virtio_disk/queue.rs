//! Request processing: walking a descriptor chain, the DMA it issues, and
//! completing the request into the used ring.

use crate::common::PhysAddr;
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::handle::HandleCtx;
use crate::sim::packet::{AccessSize, MemOp, Packet, WriteData, WriteOrigin};
use std::collections::VecDeque;

use super::VirtioBlock;
use super::{
    DESC_OFFSET_ADDR, DESC_OFFSET_FLAGS, DESC_OFFSET_LEN, DESC_OFFSET_NEXT, DESC_SIZE, DmaAccess,
    DmaJob, SECTOR_SIZE, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE, line_chunks,
};

impl VirtioBlock {
    /// Reads `len` bytes from system RAM at physical address `addr` via DMA.
    /// Returns zeroed bytes if the address is out of bounds.
    pub(super) fn dma_read(&self, addr: u64, len: usize) -> Vec<u8> {
        if addr < self.ram_base {
            return vec![0; len];
        }
        let offset = (addr - self.ram_base) as usize;

        if offset >= self.ram.len() || offset + len > self.ram.len() {
            return vec![0; len];
        }

        self.ram.read_slice(offset, len).to_vec()
    }

    pub(super) fn dma_read_u16(&self, addr: u64) -> u16 {
        let b = self.dma_read(addr, 2);
        u16::from_le_bytes([b[0], b[1]])
    }

    pub(super) fn dma_read_u32(&self, addr: u64) -> u32 {
        let b = self.dma_read(addr, 4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    pub(super) fn dma_read_u64(&self, addr: u64) -> u64 {
        let b = self.dma_read(addr, 8);
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }

    /// Writes `data` to system RAM at physical address `addr` via DMA.
    pub(super) fn dma_write(&mut self, addr: u64, data: &[u8]) {
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
    /// Begins the next available request, if one is waiting and none is
    /// in flight: its DMA is issued phase by phase and it completes when
    /// the last transfer has returned.
    pub(super) fn start_next_request(&mut self, ctx: &mut HandleCtx<'_>) {
        while self.job.is_none() {
            let Some((head_idx, ring_offset)) = self.next_available_chain() else { return };
            let phases = self.plan_request(head_idx, ring_offset);
            tracing::trace!(
                target: "rvsim::dma",
                cycle = ctx.cycle,
                head_idx,
                transfers = ?phases.iter().map(Vec::len).collect::<Vec<_>>(),
                "virtio: request started"
            );
            self.job = Some(DmaJob { head_idx, phases, outstanding: Vec::new() });
            self.issue_phase(ctx);
        }
    }

    /// Takes the next chain the driver made available: its head index and
    /// its slot's offset in the ring. Chains with an invalid head are
    /// skipped.
    pub(super) fn next_available_chain(&mut self) -> Option<(u16, u64)> {
        loop {
            if self.queue_num == 0 {
                return None;
            }
            let avail_addr = self.avail_addr();
            let avail_idx = self.dma_read_u16(avail_addr + 2);
            if self.last_avail_idx == avail_idx {
                return None;
            }
            let ring_offset = 4 + (self.last_avail_idx as u64 % self.queue_num as u64) * 2;
            let head_idx = self.dma_read_u16(avail_addr + ring_offset);
            self.last_avail_idx = self.last_avail_idx.wrapping_add(1);
            if head_idx as u32 >= self.queue_num {
                println!(
                    "[VirtIO] Error: Head descriptor index {} out of bounds (Queue Size {})",
                    head_idx, self.queue_num
                );
                continue;
            }
            return Some((head_idx, ring_offset));
        }
    }

    /// Issues the in-flight request's next DMA phase, or completes the
    /// request when none is left.
    pub(super) fn issue_phase(&mut self, ctx: &mut HandleCtx<'_>) {
        loop {
            let Some(job) = self.job.as_mut() else { return };
            let Some(phase) = job.phases.pop_front() else {
                let head_idx = job.head_idx;
                self.job = None;
                self.complete_request(head_idx);
                tracing::trace!(target: "rvsim::dma", cycle = ctx.cycle, head_idx, "virtio: request completed");
                return;
            };
            if phase.is_empty() {
                continue;
            }
            let ComponentId::Device(device) = ctx.self_id else {
                tracing::trace!(target: "rvsim::dma", cycle = ctx.cycle, self_id = ?ctx.self_id, "virtio: no device id, DMA not issued");
                return;
            };
            tracing::trace!(target: "rvsim::dma", cycle = ctx.cycle, transfers = phase.len(), "virtio: phase issued");
            for access in phase {
                let req_id = ReqId::for_device(device, self.next_dma_seq);
                self.next_dma_seq = self.next_dma_seq.wrapping_add(1);
                job.outstanding.push(req_id);
                let op = if access.write {
                    MemOp::Write { data: WriteData::Small(0), origin: WriteOrigin::Placed }
                } else {
                    MemOp::Read
                };
                ctx.scheduler.schedule(
                    ctx.cycle,
                    ComponentId::Bus,
                    ctx.self_id,
                    Packet::MemReq {
                        req_id,
                        paddr: access.paddr,
                        vaddr: None,
                        size: access.size,
                        op,
                    },
                );
            }
            return;
        }
    }

    /// Notes a DMA transfer's completion and moves on when its phase is done.
    pub(super) fn on_dma_response(&mut self, req_id: ReqId, ctx: &mut HandleCtx<'_>) {
        let Some(job) = self.job.as_mut() else {
            tracing::trace!(target: "rvsim::dma", cycle = ctx.cycle, ?req_id, "virtio: response with no request in flight");
            return;
        };
        job.outstanding.retain(|pending| *pending != req_id);
        tracing::trace!(target: "rvsim::dma", cycle = ctx.cycle, outstanding = job.outstanding.len(), "virtio: transfer returned");
        if job.outstanding.is_empty() {
            self.issue_phase(ctx);
            self.start_next_request(ctx);
        }
    }

    /// The DMA a request needs, as the bus sees it: the ring and
    /// descriptor reads, the data moved in line-sized chunks, then the
    /// status and used-ring writes.
    pub(super) fn plan_request(&self, head_idx: u16, ring_offset: u64) -> VecDeque<Vec<DmaAccess>> {
        let avail_addr = self.avail_addr();
        let mut control = vec![
            DmaAccess::read(avail_addr + 2, AccessSize::B2),
            DmaAccess::read(avail_addr + ring_offset, AccessSize::B2),
        ];
        let descriptors = self.walk_chain(head_idx).unwrap_or_default();
        for (index, _) in descriptors.iter().enumerate() {
            let desc = self.desc_addr() + index as u64 * DESC_SIZE;
            control.push(DmaAccess::read(desc, AccessSize::B8));
            control.push(DmaAccess::read(desc + 8, AccessSize::B8));
        }
        let mut data = Vec::new();
        let mut completion = Vec::new();
        if descriptors.len() >= 3 {
            let (h_addr, _, _) = descriptors[0];
            control.push(DmaAccess::read(h_addr, AccessSize::B8));
            control.push(DmaAccess::read(h_addr + 8, AccessSize::B8));
            let header = self.dma_read(h_addr, 16);
            let type_val = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let is_write = type_val == 1;
            let is_flush = type_val == 4;
            if !is_flush {
                for (d_addr, d_len, _) in &descriptors[1..descriptors.len() - 1] {
                    data.extend(line_chunks(*d_addr, u64::from(*d_len), !is_write));
                }
            }
            let (s_addr, _, _) = descriptors[descriptors.len() - 1];
            completion.push(DmaAccess::write(s_addr, AccessSize::B1));
        }
        let used_addr = self.used_addr();
        let current_used = self.dma_read_u16(used_addr + 2);
        let used_elem = used_addr + 4 + (current_used as u64 % self.queue_num as u64) * 8;
        completion.push(DmaAccess::write(used_elem, AccessSize::B8));
        completion.push(DmaAccess::write(used_addr + 2, AccessSize::B2));
        VecDeque::from([control, data, completion])
    }

    /// Follows a descriptor chain; `Err` names an index outside the queue.
    pub(super) fn walk_chain(&self, head_idx: u16) -> Result<Vec<(u64, u32, u16)>, u16> {
        let desc_addr = self.desc_addr();
        let mut current_idx = head_idx;
        let mut descriptors = Vec::new();
        loop {
            if current_idx as u32 >= self.queue_num {
                return Err(current_idx);
            }
            let addr_offset = desc_addr + (current_idx as u64 * DESC_SIZE);
            let addr = self.dma_read_u64(addr_offset + DESC_OFFSET_ADDR);
            let len = self.dma_read_u32(addr_offset + DESC_OFFSET_LEN);
            let flags = self.dma_read_u16(addr_offset + DESC_OFFSET_FLAGS);
            let next = self.dma_read_u16(addr_offset + DESC_OFFSET_NEXT);
            descriptors.push((addr, len, flags));
            if (flags & VRING_DESC_F_NEXT) == 0 {
                return Ok(descriptors);
            }
            current_idx = next;
        }
    }

    /// Performs a request whose DMA has finished: moves the data, writes
    /// the status and the used ring, and raises the interrupt.
    pub(super) fn complete_request(&mut self, head_idx: u16) {
        let descriptors = match self.walk_chain(head_idx) {
            Ok(descriptors) => descriptors,
            Err(bad_idx) => {
                println!(
                    "[VirtIO] Error: Descriptor index {} out of bounds (Queue Size {})",
                    bad_idx, self.queue_num
                );
                Vec::new()
            }
        };

        let mut len_written = 0;
        if descriptors.len() >= 3 {
            let (h_addr, _, _) = descriptors[0];
            let header = self.dma_read(h_addr, 16);
            let type_val = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let sector = u64::from_le_bytes([
                header[8], header[9], header[10], header[11], header[12], header[13], header[14],
                header[15],
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
                        let first = current_disk_offset as u64 / SECTOR_SIZE;
                        let last = (current_disk_offset + data.len()).div_ceil(SECTOR_SIZE as usize)
                            as u64;
                        self.written.extend(first..last);
                    }
                    current_disk_offset += *d_len as usize;
                    len_written += *d_len;
                }
            } else {
                for (d_addr, d_len, d_flags) in &descriptors[1..descriptors.len() - 1] {
                    if (d_flags & VRING_DESC_F_WRITE) != 0
                        && sector_offset + current_offset < self.disk_image.len()
                    {
                        let available = self.disk_image.len() - (sector_offset + current_offset);
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

        let used_addr = self.used_addr();
        let used_idx_addr = used_addr + 2;
        let current_used = self.dma_read_u16(used_idx_addr);
        let used_elem = used_addr + 4 + (current_used as u64 % self.queue_num as u64) * 8;
        self.dma_write(used_elem, &u32::from(head_idx).to_le_bytes());
        self.dma_write(used_elem + 4, &len_written.to_le_bytes());
        self.dma_write(used_idx_addr, &current_used.wrapping_add(1).to_le_bytes());
        self.interrupt_status |= 1;
    }

    pub(super) const fn desc_addr(&self) -> u64 {
        ((self.queue_desc_high as u64) << 32) | (self.queue_desc_low as u64)
    }

    pub(super) const fn avail_addr(&self) -> u64 {
        ((self.queue_avail_high as u64) << 32) | (self.queue_avail_low as u64)
    }

    pub(super) const fn used_addr(&self) -> u64 {
        ((self.queue_used_high as u64) << 32) | (self.queue_used_low as u64)
    }
}
