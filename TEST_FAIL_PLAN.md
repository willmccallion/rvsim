# Test Failure Debug Plan

**186 failures** across 7102 tests. Grouped into 6 distinct bugs, ordered by impact.

## Build & Run

```bash
make build && rvsim scripts/run_riscv_tests.py
```

Single test (for quick iteration):
```bash
rvsim -c "
from rvsim import *
cfg = Config(width=4, backend=Backend.OutOfOrder(),
             l1d=Cache('32KB', ways=4, latency=1, mshr_count=8,
                        prefetcher=Prefetcher.Stride(degree=1, table_size=64)))
sim = Simulator().config(cfg).binary('software/riscv-tests/isa/rv64uf-p-fadd')
print('exit:', sim.run(limit=500000, stats_sections=None))
"
```

Enable tracing for diagnosis:
```bash
rvsim -c "
from rvsim import *
cfg = Config(width=2, backend=Backend.InOrder(), trace=True)
sim = Simulator().config(cfg).binary('software/riscv-tests/isa/rv64mi-p-illegal')
print('exit:', sim.run(limit=10000, stats_sections=None))
" 2>&1 | tail -200
```

**After each fix**: run full suite, confirm failure count decreased, **stop for commit**, then proceed to next bug.

---

## Bug 1: FP tests + fence_i fail on all configs with MSHRs (~143 failures)

### Symptoms
- **Tests**: rv64uf-p-fadd, fmadd, fcvt_w, fdiv, rv64ud-p-fadd, fcvt, fcvt_w, fdiv, fmadd, rv64ui-p-fence_i
- **Exit codes**: 2 (test case 1 failed), 669 (timeout/trap)
- **Configs that FAIL**: every config with `mshr_count > 0` — mshr-1, mshr-16, l2-mshrs, tiny-l1, large-cache, direct-map, high-assoc, slow-l1, no-pf, stream-pf, tagged-pf, aggressive-pf, plru, fifo, mru, random, cortex-a72
- **Configs that PASS**: default `o3 w4` (`mshr_count=0`, blocking cache), blocking-l1d, no-l2, all inorder configs

### Key Observation
The ONLY architectural difference between passing and failing configs is the non-blocking cache (MSHR) path. Default config uses `mshr_count=0` (blocking), all failing configs use `mshr_count > 0`.

### Files to Investigate
- `crates/hardware/src/core/pipeline/backend/shared/memory1.rs` — MSHR request path (lines 156-311)
- `crates/hardware/src/core/pipeline/backend/o3/mod.rs` — MSHR completion handling (lines 236-261), speculative wakeup (lines 511-519), cancel (lines 346-348)
- `crates/hardware/src/core/pipeline/backend/shared/commit.rs:232-236` — fence.i MSHR flush at commit
- `crates/hardware/src/core/pipeline/backend/o3/issue_queue.rs` — speculative wakeup/cancel logic (lines 155-211)
- `crates/hardware/src/core/units/cache/mshr.rs` — MSHR file (flush, drain_completions)

### Debug Steps
1. Run `rv64uf-p-fadd` with `trace=True` on both `o3 w4` (pass, mshr=0) and `o3 w4 no-pf` (fail, mshr=8). Compare traces to find exact divergence point.
2. Look for: loads parked in MSHRs whose ROB entries never complete (stuck `Issued`), speculative wakeups that are never cancelled or confirmed, fence.i flushing MSHRs that contain older loads.
3. Check the FP test init sequence: `csrwi mstatus,0` then `csrs mstatus, 0x2000` (sets FS=Initial) then `mret` to test code. If the `mret` redirect races with MSHR processing, FS state could be stale when first FP instruction executes.
4. Check whether `fence.i` commit-time `l1d_mshrs.flush()` destroys parked loads whose ROB entries remain `Issued` forever.

### Likely Root Cause Candidates
- **A)** `commit.rs:236` — `l1d_mshrs.flush()` on fence.i commit destroys older in-flight loads, deadlocking ROB
- **B)** Speculative wakeup race: load issues → speculative wakeup → cancel arrives too late (dependent already executed with garbage)
- **C)** MSHR completion timing: parked entry enters memory2 with `complete_cycle = now`, but memory2 reads stale data because cache line installation evicted something needed
- **D)** The test initialization sequence (CSR writes to mstatus/fcsr + mret) interacts badly with MSHR-enabled pipeline timing

### Verification
Fix, rebuild, run full suite. Expect ~143 fewer failures. **STOP and commit.**

---

## Bug 2: Integer div/rem timeout under resource pressure (~25 failures)

### Symptoms
- **Tests**: rv64um-p-div, divu, divw, divuw, rem, remu, remw, remuw
- **Exit codes**: 668 (500k cycle timeout), 669
- **Configs that FAIL**: o3 w8 (all div/rem), o3 w4 small-rob (div, rem, remu), o3 w4 tight-prf (all div/rem)
- **Configs that PASS**: o3 w4 (default), o3 w1/w2/w3, all inorder

### Key Observation
Fails only under resource pressure: wide issue (w8), small ROB (16), or tight PRF (96). IntDiv is non-pipelined with 35-cycle latency and only 1 unit.

### Files to Investigate
- `crates/hardware/src/core/pipeline/backend/o3/mod.rs:461-474` — FU structural stall re-dispatch
- `crates/hardware/src/core/pipeline/backend/o3/fu_pool.rs` — FU pool acquire/has_free logic
- `crates/hardware/src/core/pipeline/backend/o3/issue_queue.rs:251-358` — select() logic
- `crates/hardware/src/core/pipeline/backend/o3/mod.rs:606-618` — can_accept() resource gating

### Debug Steps
1. Run `rv64um-p-div` with `trace=True` on `o3 w8`. Look for: repeated FU stall messages, IQ dispatch failures, ROB filling up and never draining.
2. Check if `dispatch()` returns false (IQ full) during re-dispatch after FU stall (mod.rs:470-473). If so, the instruction is silently lost.
3. Count how many div instructions are selected per cycle vs how many can actually execute (should be 1 per 35 cycles).
4. Check if the pipeline deadlocks: all ROB entries are divs waiting for the single FU, no new instructions can rename (PRF/ROB full), no divs can complete.

### Likely Root Cause Candidates
- **A)** Re-dispatch after FU stall fails when IQ is full — instruction lost, ROB entry stuck `Issued` forever
- **B)** With w8: select() picks 8 ready divs, only 1 executes, 7 re-dispatched. Next cycle repeats. Meanwhile rename stalls because IQ is full of re-dispatched divs, starving non-div instructions.
- **C)** With tight-prf: div occupies a physical register for 35 cycles, exhausting the PRF (96 - 64 arch = only 32 rename slots), blocking rename entirely

### Verification
Fix, rebuild, run full suite. Expect ~25 fewer failures. **STOP and commit.**

---

## Bug 3: rv64mi-p-illegal fails at pipeline width 2-3 (6 failures)

### Symptoms
- **Test**: rv64mi-p-illegal only
- **Exit code**: 2 (test case 1 failed)
- **Configs that FAIL**: inorder w2, inorder w3, o3 w2, o3 w3, ref cortex-a72 (w3), ref p550 (w3)
- **Configs that PASS**: all w1, w4, w8

### Key Observation
Fails on BOTH inorder and o3 at width 2-3, passes at width 1 and width 4+. Bug is in the shared frontend (fetch/decode), not the backend.

### Files to Investigate
- `crates/hardware/src/core/pipeline/frontend/decode.rs:355-470` — decode_stage bundle processing
- `crates/hardware/src/core/pipeline/frontend/fetch2.rs` — fetch2_stage instruction fetch
- `crates/hardware/src/core/pipeline/frontend/fetch1.rs` — PC generation for width instructions
- `software/riscv-tests/isa/rv64mi-p-illegal.dump` — the actual test sequence

### Debug Steps
1. Read the `rv64mi-p-illegal.dump` to understand what illegal instruction encoding is used and what the test expects (trap handler checks mcause, mtval).
2. Run with `trace=True` at width=1 (pass) and width=2 (fail). Diff the traces to find exact divergence.
3. Focus on: does the illegal instruction get decoded correctly? Does the trap propagate through the pipeline? Is `mtval` (the illegal instruction bits) correctly set?
4. Check intra-bundle hazard logic (decode.rs:393-408): does the illegal instruction's `ControlSignals::default()` incorrectly trigger a hazard check that prevents it from being decoded in the same bundle?
5. Check if width 2-3 causes the illegal instruction to land at a specific bundle position that triggers incorrect behavior (e.g., position 1 in a 2-wide bundle).

### Likely Root Cause Candidates
- **A)** Intra-bundle hazard false positive: the default ControlSignals for an illegal instruction has `rs1=0, rs2=0` which might collide with a preceding instruction's `rd=0` write
- **B)** Decode breaks early on the illegal instruction trap but doesn't properly drain the bundle, causing the next cycle to re-process stale entries
- **C)** The `bundle_writes` tracking interacts with the illegal instruction's `d.rd` field (which is extracted from the raw instruction bits even though it's illegal), creating a false WAW hazard at certain widths
- **D)** Fetch2 compressed instruction detection: the illegal instruction encoding at certain PC alignments is misidentified as compressed at width 2-3

### Verification
Fix, rebuild, test at all widths (1-8) on both inorder and o3. Expect 6 fewer failures. **STOP and commit.**

---

## Bug 4: rv64ui-p-ma_data with fu-slow and no-caches (2 failures)

### Symptoms
- **Test**: rv64ui-p-ma_data only
- **Exit code**: 1 (fatal trap in direct mode)
- **Configs that FAIL**: o3 w4 fu-slow (IntAlu latency=4), o3 w4 no-caches (l1i=None, l1d=None, l2=None)
- **Configs that PASS**: all other o3 and inorder configs

### Key Observation
`ma_data` tests misaligned loads/stores which should SUCCEED (not trap). Exit=1 means a fatal trap occurred. Only fails when memory latency is very high (slow FUs or no caches).

### Files to Investigate
- `crates/hardware/src/core/pipeline/backend/shared/memory1.rs:82-87` — unaligned latency penalty
- `crates/hardware/src/core/units/lsu/unaligned.rs` — alignment check and split functions
- `crates/hardware/src/core/pipeline/backend/shared/memory2.rs:182-236` — load data read (uses `read_unaligned()`)
- `crates/hardware/src/core/cpu/memory.rs:103-170` — simulate_memory_access (no-cache path)
- `software/riscv-tests/isa/rv64ui-p-ma_data.dump` — the test sequence

### Debug Steps
1. Read the `ma_data.dump` to understand exactly which misaligned accesses are tested and what traps are expected (if any).
2. Run with `trace=True` on `no-caches` config. Look for: unexpected trap (what kind?), misaligned address that causes a bus error or page fault.
3. Check if misaligned accesses that cross a page boundary cause a double fault when TLB is cold (no-caches path).
4. Check the `fu-slow` config: IntAlu latency=4 means address calculation takes 4 cycles. Does this cause the memory1 stage to see an incorrect address due to a timing issue?
5. Check if the `no-caches` path at `memory1.rs:312-332` correctly handles misaligned accesses (the latency calculation may not account for cross-line penalties when there's no cache).

### Likely Root Cause Candidates
- **A)** No-caches: `simulate_memory_access()` doesn't handle unaligned addresses that cross the bus width boundary, causing a bus fault
- **B)** fu-slow: high IntAlu latency causes the complete_cycle calculation to exceed some internal timeout, triggering a watchdog trap
- **C)** Misaligned access crosses a page boundary, and without caches the TLB miss handling takes a different code path that doesn't support misalignment

### Verification
Fix, rebuild, run the two specific configs. Expect 2 fewer failures. **STOP and commit.**

---

## Bug 5: FP divide/sqrt timeout (~additional fdiv failures)

### Symptoms
- **Tests**: rv64uf-p-fdiv, rv64ud-p-fdiv (also rv64ud-p-fcvt on some configs)
- **Exit code**: 669 (timeout/trap) — different from the exit=2 of other FP tests in Bug 1
- **Configs**: same MSHR configs as Bug 1, plus o3 w4 small-rob

### Key Observation
These may be a sub-issue of Bug 1 (MSHR path) or a separate issue with long-latency FP div/sqrt (21-cycle latency, non-pipelined). The `small-rob` config only fails fdiv (not fadd/fmadd), suggesting FP div has a unique resource pressure issue similar to Bug 2 (integer div).

### Debug Steps
1. After fixing Bug 1, re-run to see if fdiv failures on MSHR configs are resolved.
2. If fdiv still fails on small-rob but other FP tests pass, investigate the FpDivSqrt FU handling — same re-dispatch issue as Bug 2 but for the FP pipeline.
3. Check `fu_pool.rs` for how FpDivSqrt is classified and whether it's correctly marked as non-pipelined.

### Files to Investigate
- `crates/hardware/src/core/pipeline/backend/o3/fu_pool.rs` — FpDivSqrt FU type
- Same files as Bug 2 (re-dispatch logic)

### Verification
Fix (if needed after Bug 1), rebuild, run suite. **STOP and commit.**

---

## Bug 6: FP edge cases — fcvt on slow-l1, fcvt_w on no-pf (~3 unique failures)

### Symptoms
- `rv64uf-p-fcvt` fails ONLY on `o3 w4 slow-l1` (exit=669)
- `rv64uf-p-fcvt_w` and `rv64ud-p-fcvt_w` fail on `o3 w4 no-pf` with exit=6 (test case 3) instead of exit=2

### Key Observation
These have different exit codes from the bulk Bug 1 failures (exit=6 vs exit=2), suggesting they may be distinct issues. The `no-pf` config has MSHRs but no prefetcher, and `slow-l1` has 4-cycle L1 latency. Both have `mshr_count > 0`.

### Debug Steps
1. After fixing Bug 1, re-run these specific tests. If they pass, done.
2. If `no-pf` still fails with exit=6: run with tracing. Exit=6 means test case 3 failed (6>>1=3). Look at test case 3 in the fcvt_w test to see what specific conversion it tests.
3. Check NaN-boxing in the MSHR completion path: when a 32-bit FP load completes via MSHR, does memory2 properly NaN-box it (`ld |= 0xFFFF_FFFF_0000_0000`)?

### Files to Investigate
- `crates/hardware/src/core/pipeline/backend/shared/memory2.rs:232-235` — NaN-boxing for FP loads
- `crates/hardware/src/core/units/fpu/mod.rs` — FP conversion logic
- `crates/hardware/src/core/units/fpu/nan_handling.rs` — NaN-boxing utilities

### Verification
Fix (if needed after Bug 1), rebuild, run suite. **STOP and commit.**

---

## Execution Order Summary

| Step | Bug | Failing Tests | Expected Fix Count | Strategy |
|------|-----|--------------|-------------------|----------|
| 1 | MSHR path + fence.i + FP loads | fadd, fmadd, fcvt_w, fcvt, fdiv, fence_i | ~143 | Trace-based diagnosis of MSHR path |
| 2 | Int div/rem resource starvation | div, divu, divw, rem, remu, etc. | ~25 | Fix FU stall re-dispatch logic |
| 3 | Illegal instruction at width 2-3 | illegal | 6 | Compare w1 vs w2 traces |
| 4 | Misaligned data with slow/no memory | ma_data | 2 | Trace-based diagnosis |
| 5 | FP div/sqrt (if not fixed by 1) | fdiv | ~5 | Check FpDivSqrt FU handling |
| 6 | FP convert edge cases (if not fixed by 1) | fcvt, fcvt_w | ~3 | Check NaN-boxing in MSHR path |

After each fix: `make build && rvsim scripts/run_riscv_tests.py` → confirm improvement → **commit** → next bug.
