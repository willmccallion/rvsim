//! CAM-style Issue Queue for the O3 backend.
//!
//! Instructions dispatched from rename sit in the issue queue until all source
//! operands are ready. The wakeup/select logic allows out-of-order issue:
//! - **Wakeup (PRF path)**: when an instruction completes, its PhysReg is broadcast
//!   to all waiting entries, marking matching source operands as ready.
//! - **Wakeup (legacy path)**: when an instruction completes, its ROB tag is broadcast.
//! - **Select**: each cycle, the oldest entries with all operands ready are
//!   selected for execution (up to `width`).

use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rob::{Rob, RobState, RobTag};
use crate::core::pipeline::store_buffer::StoreBuffer;

/// State of a single source operand in an issue queue entry (PRF path).
#[derive(Clone, Debug, Default)]
pub struct OperandState {
    /// Which physical register provides this operand value.
    pub phys: PhysReg,
    /// ROB tag of the producer (legacy path; None when using PRF).
    pub tag: Option<RobTag>,
    /// Whether the operand value is available.
    pub ready: bool,
    /// The operand value (valid when `ready` is true).
    pub value: u64,
}

/// A single entry in the issue queue.
#[derive(Clone, Debug)]
pub struct IssueQueueEntry {
    /// The instruction from rename.
    pub entry: RenameIssueEntry,
    /// Source operand 1 state.
    pub src1: OperandState,
    /// Source operand 2 state.
    pub src2: OperandState,
    /// Source operand 3 state (FP fused multiply-add).
    pub src3: OperandState,
}

/// CAM-style issue queue with wakeup and oldest-first select.
pub struct IssueQueue {
    /// Fixed-size slot array. `None` = free slot.
    slots: Vec<Option<IssueQueueEntry>>,
    /// Maximum capacity.
    capacity: usize,
    /// Current number of occupied slots.
    count: usize,
}

impl IssueQueue {
    /// Create a new issue queue with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let mut slots = Vec::with_capacity(capacity);
        slots.resize_with(capacity, || None);
        Self {
            slots,
            capacity,
            count: 0,
        }
    }

    /// Dispatch an instruction from rename into the first free slot.
    ///
    /// For the O3 (PRF) path, resolves operands via the PRF.
    /// For the legacy (scoreboard) path, resolves operands via the ROB.
    pub fn dispatch(
        &mut self,
        entry: RenameIssueEntry,
        rob: &Rob,
        cpu: &Cpu,
        prf: Option<&PhysRegFile>,
    ) -> bool {
        if self.count >= self.capacity {
            return false;
        }

        let (src1, src2, src3) = if let Some(prf) = prf {
            // PRF path: check ready bits in the physical register file
            let s1 = resolve_operand_prf(entry.rs1, entry.ctrl.rs1_fp, entry.rs1_phys, prf, cpu);
            let s2 = resolve_operand_prf(entry.rs2, entry.ctrl.rs2_fp, entry.rs2_phys, prf, cpu);
            let s3 = if entry.ctrl.rs3_fp {
                resolve_operand_prf(entry.rs3, true, entry.rs3_phys, prf, cpu)
            } else {
                OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: 0,
                }
            };
            (s1, s2, s3)
        } else {
            // Legacy scoreboard path: check ROB completion
            let s1 = resolve_operand_legacy(entry.rs1, entry.ctrl.rs1_fp, entry.rs1_tag, rob, cpu);
            let s2 = resolve_operand_legacy(entry.rs2, entry.ctrl.rs2_fp, entry.rs2_tag, rob, cpu);
            let s3 = if entry.ctrl.rs3_fp {
                resolve_operand_legacy(entry.rs3, true, entry.rs3_tag, rob, cpu)
            } else {
                OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: 0,
                }
            };
            (s1, s2, s3)
        };

        let iq_entry = IssueQueueEntry {
            entry,
            src1,
            src2,
            src3,
        };

        // Find first free slot
        for slot in &mut self.slots {
            if slot.is_none() {
                *slot = Some(iq_entry);
                self.count += 1;
                return true;
            }
        }

        unreachable!("count < capacity but no free slot found");
    }

    /// Broadcast a completed result via physical register (PRF wakeup path).
    pub fn wakeup_phys(&mut self, p: PhysReg, value: u64) {
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.phys == p && !iq.src1.ready {
                iq.src1.ready = true;
                iq.src1.value = value;
            }
            if iq.src2.phys == p && !iq.src2.ready {
                iq.src2.ready = true;
                iq.src2.value = value;
            }
            if iq.src3.phys == p && !iq.src3.ready {
                iq.src3.ready = true;
                iq.src3.value = value;
            }
        }
    }

    /// Broadcast a completed result via ROB tag (legacy wakeup path).
    pub fn wakeup(&mut self, tag: RobTag, value: u64) {
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.tag == Some(tag) && !iq.src1.ready {
                iq.src1.ready = true;
                iq.src1.value = value;
            }
            if iq.src2.tag == Some(tag) && !iq.src2.ready {
                iq.src2.ready = true;
                iq.src2.value = value;
            }
            if iq.src3.tag == Some(tag) && !iq.src3.ready {
                iq.src3.ready = true;
                iq.src3.value = value;
            }
        }
    }

    /// Select up to `width` ready entries, oldest first (lowest rob_tag.0).
    ///
    /// Selected entries have their `rv1/rv2/rv3` fields populated from the
    /// resolved operand values. The slots are freed.
    ///
    /// Loads (mem_read) are not selected if there are older unresolved stores
    /// in the store buffer (stores whose physical address is not yet known).
    /// This prevents memory ordering violations where a load could bypass an
    /// older store to the same address.
    ///
    /// System/CSR instructions are serializing: they must not issue until all
    /// older ROB entries have completed.
    pub fn select(
        &mut self,
        width: usize,
        store_buffer: &StoreBuffer,
        rob: &Rob,
    ) -> Vec<RenameIssueEntry> {
        // Collect indices of all ready entries
        let mut ready_indices: Vec<usize> = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(iq) = slot {
                // Faulted instructions don't need operands — always ready
                let all_ready =
                    iq.entry.trap.is_some() || (iq.src1.ready && iq.src2.ready && iq.src3.ready);
                if all_ready {
                    // Loads must wait for all older stores to have resolved addresses
                    if iq.entry.ctrl.mem_read
                        && store_buffer.has_unresolved_store_before(iq.entry.rob_tag)
                    {
                        continue;
                    }
                    // System/CSR instructions are serializing: wait for all
                    // older instructions to complete before issuing.
                    if iq.entry.ctrl.is_system && !rob.all_before_completed(iq.entry.rob_tag) {
                        continue;
                    }
                    ready_indices.push(i);
                }
            }
        }

        // Sort by rob_tag (oldest first = lowest tag value)
        ready_indices.sort_by_key(|&i| self.slots[i].as_ref().unwrap().entry.rob_tag.0);

        // Take up to `width`
        let take = ready_indices.len().min(width);
        let mut result = Vec::with_capacity(take);
        for &idx in &ready_indices[..take] {
            let iq = self.slots[idx].take().unwrap();
            self.count -= 1;

            let mut entry = iq.entry;
            // Populate operand values from resolved state
            if entry.trap.is_none() {
                entry.rv1 = iq.src1.value;
                entry.rv2 = iq.src2.value;
                entry.rv3 = iq.src3.value;
            }
            result.push(entry);
        }

        result
    }

    /// Number of free slots available for dispatch.
    pub fn available_slots(&self) -> usize {
        self.capacity - self.count
    }

    /// Flush all entries.
    pub fn flush(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.count = 0;
    }

    /// Flush entries with `rob_tag.0 > keep_tag.0`.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for slot in &mut self.slots {
            if let Some(iq) = slot
                && iq.entry.rob_tag.0 > keep_tag.0
            {
                *slot = None;
                self.count -= 1;
            }
        }
    }

    /// Return a snapshot of all entries in the queue (sorted by rob_tag, oldest first).
    pub fn queue_snapshot(&self) -> Vec<RenameIssueEntry> {
        let mut entries: Vec<&IssueQueueEntry> =
            self.slots.iter().filter_map(|s| s.as_ref()).collect();
        entries.sort_by_key(|iq| iq.entry.rob_tag.0);
        entries.into_iter().map(|iq| iq.entry.clone()).collect()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Current number of entries.
    pub fn len(&self) -> usize {
        self.count
    }
}

/// Resolve an operand via the PRF (O3 path).
fn resolve_operand_prf(
    reg: usize,
    is_fp: bool,
    phys: PhysReg,
    prf: &PhysRegFile,
    _cpu: &Cpu,
) -> OperandState {
    // x0 is hardwired zero
    if !is_fp && reg == 0 {
        return OperandState {
            phys: PhysReg(0),
            tag: None,
            ready: true,
            value: 0,
        };
    }

    if prf.is_ready(phys) {
        OperandState {
            phys,
            tag: None,
            ready: true,
            value: prf.read(phys),
        }
    } else {
        // Not ready yet — will be woken up by wakeup_phys
        // For x0 (phys == PhysReg(0)), always ready
        if phys.0 == 0 {
            return OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
            };
        }
        OperandState {
            phys,
            tag: None,
            ready: false,
            value: 0,
        }
    }
}

/// Resolve an operand's initial state at dispatch time (legacy scoreboard path).
fn resolve_operand_legacy(
    reg: usize,
    is_fp: bool,
    tag: Option<RobTag>,
    rob: &Rob,
    cpu: &Cpu,
) -> OperandState {
    // x0 is hardwired zero
    if !is_fp && reg == 0 {
        return OperandState {
            phys: PhysReg(0),
            tag: None,
            ready: true,
            value: 0,
        };
    }

    match tag {
        None => {
            // No in-flight producer — read from architectural register file
            let value = if is_fp {
                cpu.regs.read_f(reg)
            } else {
                cpu.regs.read(reg)
            };
            OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value,
            }
        }
        Some(t) => {
            // Check if ROB entry has completed
            match rob.find_entry(t) {
                Some(entry) if entry.state == RobState::Completed => OperandState {
                    phys: PhysReg(0),
                    tag: Some(t),
                    ready: true,
                    value: entry.result,
                },
                Some(_) => OperandState {
                    phys: PhysReg(0),
                    tag: Some(t),
                    ready: false,
                    value: 0,
                },
                None => {
                    // ROB entry already committed — read from register file
                    let value = if is_fp {
                        cpu.regs.read_f(reg)
                    } else {
                        cpu.regs.read(reg)
                    };
                    OperandState {
                        phys: PhysReg(0),
                        tag: None,
                        ready: true,
                        value,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::pipeline::latches::RenameIssueEntry;
    use crate::core::pipeline::prf::PhysReg;
    use crate::core::pipeline::rob::RobTag;
    use crate::core::pipeline::signals::ControlSignals;

    fn make_entry(rob_tag: u32) -> RenameIssueEntry {
        RenameIssueEntry {
            rob_tag: RobTag(rob_tag),
            pc: 0x1000 + (rob_tag as u64) * 4,
            inst: 0x13, // NOP
            inst_size: 4,
            rs1: 0,
            rs2: 0,
            rs3: 0,
            rd: 1,
            imm: 0,
            rv1: 0,
            rv2: 0,
            rv3: 0,
            rs1_tag: None,
            rs2_tag: None,
            rs3_tag: None,
            rs1_phys: PhysReg(0),
            rs2_phys: PhysReg(0),
            rs3_phys: PhysReg(0),
            rd_phys: PhysReg(0),
            ctrl: ControlSignals::default(),
            trap: None,
            exception_stage: None,
            pred_taken: false,
            pred_target: 0,
            ghr_snapshot: 0,
        }
    }

    #[test]
    fn test_new_empty() {
        let iq = IssueQueue::new(16);
        assert!(iq.is_empty());
        assert_eq!(iq.available_slots(), 16);
    }

    #[test]
    fn test_dispatch_and_select_ready() {
        let mut iq = IssueQueue::new(16);

        // Manually insert a ready entry
        iq.slots[0] = Some(IssueQueueEntry {
            entry: make_entry(1),
            src1: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 42,
            },
            src2: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 10,
            },
            src3: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
            },
        });
        iq.count = 1;

        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].rob_tag.0, 1);
        assert_eq!(selected[0].rv1, 42);
        assert_eq!(selected[0].rv2, 10);
        assert!(iq.is_empty());
    }

    #[test]
    fn test_wakeup_phys_chain() {
        let mut iq = IssueQueue::new(16);
        let p5 = PhysReg(5);

        // Entry depends on phys reg 5
        let entry = make_entry(10);
        iq.slots[0] = Some(IssueQueueEntry {
            entry,
            src1: OperandState {
                phys: p5,
                tag: None,
                ready: false,
                value: 0,
            },
            src2: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
            },
            src3: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
            },
        });
        iq.count = 1;

        // Not ready yet
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64));
        assert_eq!(selected.len(), 0);

        // Wakeup with phys reg 5
        iq.wakeup_phys(p5, 999);

        // Now should be selectable
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].rv1, 999);
    }

    #[test]
    fn test_wakeup_legacy_chain() {
        let mut iq = IssueQueue::new(16);

        // Entry depends on tag 5
        let entry = make_entry(10);
        iq.slots[0] = Some(IssueQueueEntry {
            entry,
            src1: OperandState {
                phys: PhysReg(0),
                tag: Some(RobTag(5)),
                ready: false,
                value: 0,
            },
            src2: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
            },
            src3: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
            },
        });
        iq.count = 1;

        // Wakeup with tag 5
        iq.wakeup(RobTag(5), 999);

        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].rv1, 999);
    }

    #[test]
    fn test_oldest_first_select() {
        let mut iq = IssueQueue::new(16);

        // Insert entries with tags 3, 1, 2 in random slot order
        for (slot, tag) in [(2, 3u32), (0, 1), (1, 2)] {
            iq.slots[slot] = Some(IssueQueueEntry {
                entry: make_entry(tag),
                src1: OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: tag as u64,
                },
                src2: OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: 0,
                },
                src3: OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: 0,
                },
            });
        }
        iq.count = 3;

        // Select width=2 should get tags 1 and 2 (oldest first)
        let selected = iq.select(2, &StoreBuffer::new(16), &Rob::new(64));
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].rob_tag.0, 1);
        assert_eq!(selected[1].rob_tag.0, 2);
        assert_eq!(iq.len(), 1);

        // Remaining is tag 3
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64));
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].rob_tag.0, 3);
    }

    #[test]
    fn test_flush() {
        let mut iq = IssueQueue::new(16);
        iq.slots[0] = Some(IssueQueueEntry {
            entry: make_entry(1),
            src1: OperandState::default(),
            src2: OperandState::default(),
            src3: OperandState::default(),
        });
        iq.slots[5] = Some(IssueQueueEntry {
            entry: make_entry(2),
            src1: OperandState::default(),
            src2: OperandState::default(),
            src3: OperandState::default(),
        });
        iq.count = 2;

        iq.flush();
        assert!(iq.is_empty());
        assert_eq!(iq.available_slots(), 16);
    }

    #[test]
    fn test_flush_after() {
        let mut iq = IssueQueue::new(16);
        for (slot, tag) in [(0, 1u32), (1, 2), (2, 3), (3, 4)] {
            iq.slots[slot] = Some(IssueQueueEntry {
                entry: make_entry(tag),
                src1: OperandState::default(),
                src2: OperandState::default(),
                src3: OperandState::default(),
            });
        }
        iq.count = 4;

        // Keep tags <= 2
        iq.flush_after(RobTag(2));
        assert_eq!(iq.len(), 2);

        let snap = iq.queue_snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].rob_tag.0, 1);
        assert_eq!(snap[1].rob_tag.0, 2);
    }

    #[test]
    fn test_queue_snapshot_sorted() {
        let mut iq = IssueQueue::new(16);
        // Insert in reverse order
        for (slot, tag) in [(0, 5u32), (1, 3), (2, 1)] {
            iq.slots[slot] = Some(IssueQueueEntry {
                entry: make_entry(tag),
                src1: OperandState::default(),
                src2: OperandState::default(),
                src3: OperandState::default(),
            });
        }
        iq.count = 3;

        let snap = iq.queue_snapshot();
        assert_eq!(snap.len(), 3);
        assert_eq!(snap[0].rob_tag.0, 1);
        assert_eq!(snap[1].rob_tag.0, 3);
        assert_eq!(snap[2].rob_tag.0, 5);
    }
}
