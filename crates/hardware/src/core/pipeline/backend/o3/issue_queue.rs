//! CAM-style Issue Queue for the O3 backend.
//!
//! Instructions dispatched from rename sit in the issue queue until all source
//! operands are ready. The wakeup/select logic allows out-of-order issue:
//! - **Wakeup (PRF path)**: when an instruction completes, its `PhysReg` is broadcast
//!   to all waiting entries, marking matching source operands as ready.
//! - **Wakeup (legacy path)**: when an instruction completes, its ROB tag is broadcast.
//! - **Select**: each cycle, the oldest entries with all operands ready are
//!   selected for execution (up to `width`).

use crate::common::RegIdx;
use crate::core::Cpu;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::prf::{PhysReg, PhysRegFile};
use crate::core::pipeline::rob::{Rob, RobState, RobTag};
use crate::core::pipeline::signals::SystemOp;
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
    /// Whether this operand was speculatively woken (load hit speculation).
    /// When true, the PRF has NOT been written yet — the value in `value`
    /// is a placeholder. Must be validated against the PRF before issue.
    pub speculative: bool,
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
#[derive(Debug)]
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
        Self { slots, capacity, count: 0 }
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
                    speculative: false,
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
                    speculative: false,
                }
            };
            (s1, s2, s3)
        };

        let iq_entry = IssueQueueEntry { entry, src1, src2, src3 };

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

    /// Speculatively mark operands waiting on `p` as ready (value=0 placeholder).
    ///
    /// Used for load speculation: when a load issues, we optimistically wake
    /// dependents so they can be selected next cycle (assuming L1D hit).
    /// The PRF is NOT written — `select()` validates against PRF before issuing.
    /// If the load hits, normal writeback will write the PRF and re-wakeup with
    /// the real value. If it misses, `cancel_wakeup_phys()` reverts this.
    pub fn speculative_wakeup_phys(&mut self, p: PhysReg) {
        if p.0 == 0 {
            return;
        }
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.phys == p && !iq.src1.ready {
                iq.src1.ready = true;
                iq.src1.speculative = true;
                // value stays 0 — real value comes from PRF at select time
            }
            if iq.src2.phys == p && !iq.src2.ready {
                iq.src2.ready = true;
                iq.src2.speculative = true;
            }
            if iq.src3.phys == p && !iq.src3.ready {
                iq.src3.ready = true;
                iq.src3.speculative = true;
            }
        }
    }

    /// Cancel a speculative wakeup: revert operands waiting on `p` to not-ready.
    ///
    /// Called when a speculatively-woken load turns out to be an L1D miss.
    /// Only reverts operands whose PRF entry is still not-ready (the speculative
    /// ones). Operands that have since been written by a real wakeup are unaffected.
    pub fn cancel_wakeup_phys(&mut self, p: PhysReg, prf: &PhysRegFile) {
        if p.0 == 0 {
            return;
        }
        // Only cancel if the PRF says this register is still not ready
        // (i.e., no real wakeup has arrived yet).
        if prf.is_ready(p) {
            return;
        }
        for iq in self.slots.iter_mut().flatten() {
            if iq.src1.phys == p && iq.src1.ready && iq.src1.speculative {
                iq.src1.ready = false;
                iq.src1.speculative = false;
            }
            if iq.src2.phys == p && iq.src2.ready && iq.src2.speculative {
                iq.src2.ready = false;
                iq.src2.speculative = false;
            }
            if iq.src3.phys == p && iq.src3.ready && iq.src3.speculative {
                iq.src3.ready = false;
                iq.src3.speculative = false;
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

    /// Select up to `width` ready entries, oldest first (lowest `rob_tag.0`).
    ///
    /// Selected entries have their `rv1/rv2/rv3` fields populated from the
    /// resolved operand values. The slots are freed.
    ///
    /// When `prf` is provided, operands that were speculatively marked ready
    /// are validated: if the IQ says ready but the PRF says not-ready, the
    /// operand was speculatively woken by a load that hasn't completed yet.
    /// Such entries are skipped (not issued).
    ///
    /// Loads (`mem_read`) are not selected if there are older unresolved stores
    /// in the store buffer (stores whose physical address is not yet known).
    /// This prevents memory ordering violations where a load could bypass an
    /// older store to the same address.
    ///
    /// System/CSR instructions are serializing: they must not issue until all
    /// older ROB entries have completed.
    ///
    /// Memory port limits: at most `load_ports` loads and `store_ports` stores
    /// are issued per cycle, modeling finite LSU bandwidth.
    pub fn select(
        &mut self,
        width: usize,
        store_buffer: &StoreBuffer,
        rob: &Rob,
        load_ports: usize,
        store_ports: usize,
        prf: Option<&PhysRegFile>,
    ) -> Vec<RenameIssueEntry> {
        // Collect indices of all ready entries
        let mut ready_indices: Vec<usize> = Vec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(iq) = slot {
                // Faulted instructions don't need operands — always ready
                let all_ready =
                    iq.entry.trap.is_some() || (iq.src1.ready && iq.src2.ready && iq.src3.ready);
                // PRF validation: if an operand was speculatively woken (IQ says
                // ready) but the PRF still says not-ready, the speculative wakeup
                // hasn't been confirmed yet — treat as not ready.
                let prf_valid = prf.is_none_or(|prf| {
                    Self::prf_validated(&iq.src1, prf)
                        && Self::prf_validated(&iq.src2, prf)
                        && Self::prf_validated(&iq.src3, prf)
                });
                let all_ready = all_ready && prf_valid;
                if all_ready {
                    // Loads must wait for all older stores to have resolved addresses
                    if iq.entry.ctrl.mem_read
                        && store_buffer.has_unresolved_store_before(iq.entry.rob_tag)
                    {
                        continue;
                    }
                    // System/CSR instructions are serializing: wait for all
                    // older instructions to complete before issuing.
                    if iq.entry.ctrl.system_op != SystemOp::None
                        && !rob.all_before_completed(iq.entry.rob_tag)
                    {
                        continue;
                    }
                    // FENCE: wait for older operations matching pred bits to complete.
                    if iq.entry.ctrl.system_op == SystemOp::Fence {
                        let pred_bits = ((iq.entry.inst >> 24) & 0xF) as u8;
                        let pred_r = pred_bits & 0b0010 != 0;
                        let pred_w = pred_bits & 0b0001 != 0;
                        if !rob.fence_pred_satisfied(iq.entry.rob_tag, pred_r, pred_w) {
                            continue;
                        }
                    }
                    // Loads/stores: blocked by older in-flight FENCE with matching succ bits.
                    if (iq.entry.ctrl.mem_read || iq.entry.ctrl.mem_write)
                        && rob.has_fence_blocking(
                            iq.entry.rob_tag,
                            iq.entry.ctrl.mem_read,
                            iq.entry.ctrl.mem_write,
                        )
                    {
                        continue;
                    }
                    ready_indices.push(i);
                }
            }
        }

        // Sort by rob_tag (oldest first = lowest tag value)
        ready_indices.sort_by_key(|&i| self.slots[i].as_ref().map_or(0, |s| s.entry.rob_tag.0));

        // Take up to `width`, respecting per-type port limits
        let mut result = Vec::with_capacity(width);
        let mut loads_issued = 0usize;
        let mut stores_issued = 0usize;
        for &idx in &ready_indices {
            if result.len() >= width {
                break;
            }
            let Some(slot) = self.slots[idx].as_ref() else { continue };
            let ctrl = &slot.entry.ctrl;
            let is_load = ctrl.mem_read;
            let is_store = ctrl.mem_write;
            if is_load && loads_issued >= load_ports {
                continue; // port limit reached; entry stays in IQ
            }
            if is_store && stores_issued >= store_ports {
                continue;
            }

            let Some(iq) = self.slots[idx].take() else { continue };
            self.count -= 1;
            if is_load {
                loads_issued += 1;
            }
            if is_store {
                stores_issued += 1;
            }

            let mut entry = iq.entry;
            // Populate operand values from resolved state.
            // If PRF is available and the operand value is 0 with a non-zero
            // phys reg, read the real value from PRF (handles speculative
            // wakeup where IQ had value=0 but PRF now has the real value).
            if entry.trap.is_none() {
                entry.rv1 = Self::resolve_value(&iq.src1, prf);
                entry.rv2 = Self::resolve_value(&iq.src2, prf);
                entry.rv3 = Self::resolve_value(&iq.src3, prf);
            }
            result.push(entry);
        }

        result
    }

    /// Check whether a speculatively-ready operand is validated by the PRF.
    ///
    /// Returns `true` if the operand is genuinely ready, or was never
    /// speculatively woken. Returns `false` if it was speculatively woken
    /// and the PRF still hasn't been written (load hasn't completed yet).
    #[inline]
    fn prf_validated(src: &OperandState, prf: &PhysRegFile) -> bool {
        if !src.ready || !src.speculative {
            return true; // not ready or not speculative → no validation needed
        }
        // Speculative: check PRF — if PRF is ready, the real wakeup arrived.
        prf.is_ready(src.phys)
    }

    /// Resolve the final operand value at select time.
    ///
    /// If the operand was speculatively woken, the IQ entry has a placeholder
    /// value — read the real value from the PRF (which has been written by
    /// the real wakeup by the time we reach select).
    #[inline]
    fn resolve_value(src: &OperandState, prf: Option<&PhysRegFile>) -> u64 {
        if !src.speculative || src.phys.0 == 0 {
            return src.value;
        }
        // Speculative: PRF has the authoritative value.
        prf.map_or(src.value, |prf| prf.read(src.phys))
    }

    /// Number of free slots available for dispatch.
    pub const fn available_slots(&self) -> usize {
        self.capacity - self.count
    }

    /// Flush all entries.
    pub fn flush(&mut self) {
        for slot in &mut self.slots {
            *slot = None;
        }
        self.count = 0;
    }

    /// Flush entries newer than `keep_tag`.
    pub fn flush_after(&mut self, keep_tag: RobTag) {
        for slot in &mut self.slots {
            if let Some(iq) = slot
                && iq.entry.rob_tag.is_newer_than(keep_tag)
            {
                *slot = None;
                self.count -= 1;
            }
        }
    }

    /// Return a snapshot of all entries in the queue (sorted by `rob_tag`, oldest first).
    pub fn queue_snapshot(&self) -> Vec<RenameIssueEntry> {
        let mut entries: Vec<&IssueQueueEntry> =
            self.slots.iter().filter_map(|s| s.as_ref()).collect();
        entries.sort_by_key(|iq| iq.entry.rob_tag.0);
        entries.into_iter().map(|iq| iq.entry.clone()).collect()
    }

    /// Whether the queue is empty.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Current number of entries.
    pub const fn len(&self) -> usize {
        self.count
    }
}

/// Resolve an operand via the PRF (O3 path).
fn resolve_operand_prf(
    reg: RegIdx,
    is_fp: bool,
    phys: PhysReg,
    prf: &PhysRegFile,
    _cpu: &Cpu,
) -> OperandState {
    // x0 is hardwired zero
    if !is_fp && reg.is_zero() {
        return OperandState {
            phys: PhysReg(0),
            tag: None,
            ready: true,
            value: 0,
            speculative: false,
        };
    }

    if prf.is_ready(phys) {
        OperandState { phys, tag: None, ready: true, value: prf.read(phys), speculative: false }
    } else {
        // Not ready yet — will be woken up by wakeup_phys
        // For x0 (phys == PhysReg(0)), always ready
        if phys.0 == 0 {
            return OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
                speculative: false,
            };
        }
        OperandState { phys, tag: None, ready: false, value: 0, speculative: false }
    }
}

/// Resolve an operand's initial state at dispatch time (legacy scoreboard path).
fn resolve_operand_legacy(
    reg: RegIdx,
    is_fp: bool,
    tag: Option<RobTag>,
    rob: &Rob,
    cpu: &Cpu,
) -> OperandState {
    // x0 is hardwired zero
    if !is_fp && reg.is_zero() {
        return OperandState {
            phys: PhysReg(0),
            tag: None,
            ready: true,
            value: 0,
            speculative: false,
        };
    }

    tag.map_or_else(
        || {
            // No in-flight producer — read from architectural register file
            let value = if is_fp { cpu.regs.read_f(reg) } else { cpu.regs.read(reg) };
            OperandState { phys: PhysReg(0), tag: None, ready: true, value, speculative: false }
        },
        |t| {
            // Check if ROB entry has completed
            match rob.find_entry(t) {
                Some(entry) if entry.state == RobState::Completed => OperandState {
                    phys: PhysReg(0),
                    tag: Some(t),
                    ready: true,
                    value: entry.result,
                    speculative: false,
                },
                Some(_) => OperandState {
                    phys: PhysReg(0),
                    tag: Some(t),
                    ready: false,
                    value: 0,
                    speculative: false,
                },
                None => {
                    // ROB entry already committed — read from register file
                    let value = if is_fp { cpu.regs.read_f(reg) } else { cpu.regs.read(reg) };
                    OperandState {
                        phys: PhysReg(0),
                        tag: None,
                        ready: true,
                        value,
                        speculative: false,
                    }
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::InstSize;
    use crate::core::pipeline::latches::RenameIssueEntry;
    use crate::core::pipeline::prf::PhysReg;
    use crate::core::pipeline::rob::RobTag;
    use crate::core::pipeline::signals::ControlSignals;

    fn make_entry(rob_tag: u32) -> RenameIssueEntry {
        RenameIssueEntry {
            rob_tag: RobTag(rob_tag),
            pc: 0x1000 + (rob_tag as u64) * 4,
            inst: 0x13, // NOP
            inst_size: InstSize::Standard,
            rs1: RegIdx::new(0),
            rs2: RegIdx::new(0),
            rs3: RegIdx::new(0),
            rd: RegIdx::new(1),
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
            ras_snapshot: 0,
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
                speculative: false,
            },
            src2: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 10,
                speculative: false,
            },
            src3: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
                speculative: false,
            },
        });
        iq.count = 1;

        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
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
            src1: OperandState { phys: p5, tag: None, ready: false, value: 0, speculative: false },
            src2: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
                speculative: false,
            },
            src3: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
                speculative: false,
            },
        });
        iq.count = 1;

        // Not ready yet
        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 0);

        // Wakeup with phys reg 5
        iq.wakeup_phys(p5, 999);

        // Now should be selectable
        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
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
                speculative: false,
            },
            src2: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
                speculative: false,
            },
            src3: OperandState {
                phys: PhysReg(0),
                tag: None,
                ready: true,
                value: 0,
                speculative: false,
            },
        });
        iq.count = 1;

        // Wakeup with tag 5
        iq.wakeup(RobTag(5), 999);

        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
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
                    speculative: false,
                },
                src2: OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: 0,
                    speculative: false,
                },
                src3: OperandState {
                    phys: PhysReg(0),
                    tag: None,
                    ready: true,
                    value: 0,
                    speculative: false,
                },
            });
        }
        iq.count = 3;

        // Select width=2 should get tags 1 and 2 (oldest first)
        let selected =
            iq.select(2, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].rob_tag.0, 1);
        assert_eq!(selected[1].rob_tag.0, 2);
        assert_eq!(iq.len(), 1);

        // Remaining is tag 3
        let selected =
            iq.select(4, &StoreBuffer::new(16), &Rob::new(64), usize::MAX, usize::MAX, None);
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

    #[test]
    fn test_port_limits() {
        let mut iq = IssueQueue::new(16);

        // Insert 3 loads (tags 1, 2, 3) and 2 stores (tags 4, 5), all ready
        for (slot, tag, is_load, is_store) in [
            (0, 1u32, true, false),
            (1, 2, true, false),
            (2, 3, true, false),
            (3, 4, false, true),
            (4, 5, false, true),
        ] {
            let mut entry = make_entry(tag);
            entry.ctrl.mem_read = is_load;
            entry.ctrl.mem_write = is_store;
            iq.slots[slot] = Some(IssueQueueEntry {
                entry,
                src1: OperandState { ready: true, ..Default::default() },
                src2: OperandState { ready: true, ..Default::default() },
                src3: OperandState { ready: true, ..Default::default() },
            });
        }
        iq.count = 5;

        // With load_ports=2, store_ports=1, width=4: should get 2 loads + 1 store = 3
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64), 2, 1, None);
        assert_eq!(selected.len(), 3);
        // Oldest first: tags 1 (load), 2 (load), 4 (store)
        assert_eq!(selected[0].rob_tag.0, 1);
        assert!(selected[0].ctrl.mem_read);
        assert_eq!(selected[1].rob_tag.0, 2);
        assert!(selected[1].ctrl.mem_read);
        assert_eq!(selected[2].rob_tag.0, 4);
        assert!(selected[2].ctrl.mem_write);

        // Remaining: tag 3 (load), tag 5 (store)
        assert_eq!(iq.len(), 2);

        // Next cycle: should get remaining load + store
        let selected = iq.select(4, &StoreBuffer::new(16), &Rob::new(64), 2, 1, None);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].rob_tag.0, 3);
        assert_eq!(selected[1].rob_tag.0, 5);
        assert!(iq.is_empty());
    }
}
