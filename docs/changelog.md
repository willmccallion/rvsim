# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- The in-order backend forwards a load's value to a dependent the cycle
  memory2 delivers it, as it already did a unit's result and as Rocket
  bypasses its D-cache response into the next instruction's execute, so a
  dependent load costs the L1D latency plus one cycle rather than two
  (#210). Every in-order cycle count moves.
- Each commit-log line ends its header with `cycle <n>`: the cycle the
  instruction retired in, or the cycle a trap was taken in, so the log
  doubles as a retire trace that lines up against an RTL core's (#177).
- `Simulator.open_commit_log` writes what a reference model needs to replay
  the run: the architectural state the log starts from, each retired
  instruction's privilege mode, destination register, the CSR it wrote (as
  read back), the FP flags it raised and its load or store (virtual and
  physical address, size and value), and each trap taken. A vector
  instruction's line also gives each vector register it wrote, with the bits
  it filled under a tail- or mask-agnostic policy, and each element it loaded
  or stored. An FP destination was logged as an `x` register.
- `make lockstep` (`tests/conformance/lockstep.py`, `make lockstep-smoke`
  for the smoke configs) replays rvsim's commit log on spike one instruction
  at a time for every riscv-test, program, benchmark and RVV test on every
  single-hart pipeline config, and stops at the first instruction whose PC,
  privilege mode, destination, vector registers, CSR write, FP flags or
  memory access differs. Values the ISA leaves to the implementation
  (counters, timers, IDs, device reads, interrupt arrival, WFI wake-up, lost
  reservations, agnostic vector elements) are taken from rvsim's log. The driver is `tools/lockstep/spike_lockstep.cc`, built against the
  pinned spike. `make test-all` runs it; the smoke run does not.
- A `pmpaddr` register kept all 64 bits written to it, so bits 63:54 read
  back as written (#170). On RV64 they read as zero; only bits 53:0
  (physical address bits 55:2) are kept now.
- Debug triggers read `tdata1`'s `mcontrol` fields from the wrong bits:
  M, S and U at 13, 11 and 10 and execute, store and load at 9, 8 and 7,
  where the debug spec puts them at 6, 4 and 3 and 2, 1 and 0 (#171). A
  trigger written by software that follows the spec, such as a debugger's
  breakpoint, came back disarmed. The fields are now where the spec puts
  them.
- Writing `satp` with a MODE the hart does not support, or one above
  `paging_mode_max`, set MODE to Bare and kept the written ASID and PPN
  (#172). The privileged spec says such a write has no effect, and now it
  has none.
- The `vec_stress` example's masked-operations check failed on rvsim
  (#173): it used the mask-agnostic `__riscv_vadd_vv_i64m2_m`, whose
  inactive elements may be all ones, as rvsim makes them, and expected them
  to hold `va`. It now uses the mask-undisturbed form with `va` as the merge
  operand.
- `misa.B` is set (#174): every hart implements Zba, Zbb and Zbs, so B now
  says so, whatever `Config(isa=...)` names, and the device tree's
  `riscv,isa` gains a `b` (`rv64imafdcbv_sstc` by default). `isa` accepts
  `B`.
- A load or store trigger's breakpoint set `mtval` to the instruction's PC
  (#176). The privileged spec gives the faulting virtual address, which for
  a data trigger is the address accessed; it does now, and a CBO's trigger
  reports the address its other faults report.
- Half-precision arithmetic, scalar (Zfh) and vector (Zvfh), raised the
  invalid flag only for a signaling-NaN operand (#178). It now also raises
  it for ∞ − ∞ in an add or subtract, 0 × ∞ in a multiply, and both in a
  fused multiply-add, where 0 × ∞ is invalid even with a quiet-NaN addend.
  `rv64uzfh-p-fadd` failed on this; the riscv-tests runner now runs the
  `rv64uzfh`, `rv64uzba`, `rv64uzbb`, `rv64uzbc` and `rv64uzbs` suites,
  which were built but never run.
- The vector widening FP operations dropped the invalid flag for a
  signaling-NaN operand (#182): `vfwadd`, `vfwsub` and `vfwmul` (and their
  `.w` forms) at SEW=32 and 16, and `vfwredusum`/`vfwredosum` at SEW=32. The
  host conversion of the operand to the wider format raised it before the
  flags were cleared. The widening FMAs worked only because of where the
  compiler placed that conversion. All of them now check their operands.
- RMM (round to nearest, ties to max magnitude) rounded ties to even,
  scalar and vector, because the host FPU has no such mode (#179). An
  inexact RMM result is now rounded again in software from the operation's
  exact value. The software rounding (`exec/compute/fpu/exact.rs`) is
  property-tested against the host in the four modes the host has. The
  half-precision FMAs, scalar and vector, and the Zvfh widening FMA rounded
  twice (to binary64, then to the result format) in every mode, and now
  round once. The Zvfh widening add, subtract and multiply lost the flags
  of their final rounding to binary32 (#180).
- A vector instruction with `vstart` ≥ `vl` updated its destination (#183):
  a reduction with vl=0 wrote `vs1[0]` into `vd[0]`, and element-wise ops
  filled their tail with ones under a tail-agnostic policy. The vector spec
  says no destination element, agnostic or not, is updated then. Now none
  is, except by the instructions that still execute: those writing an `x`
  or `f` register and the whole-register moves.
- On the O3 backend a vector load could read memory from before an older
  vector store whose element addresses resolved after the load ran (#184).
  The load's micro-ops gave up their load-queue slots as they wrote back,
  so the store's ordering check missed them. A vector load now reserves its
  slots at rename, one per micro-op it expects (an element, or a window of
  an aligned unit-stride access), and its micro-ops hold them until it
  commits. A load with more micro-ops than slots (an oversize load, a
  misaligned unit-stride load, a span split at a fault) reuses its slots
  once no older memory access is in flight. Lockstep against spike stopped
  on this in the RVV `vsoxei16` and `vsseg2e32` tests.
- The vector float-to-integer conversions at a 16-bit width, `vfwcvt.x[u].f.v`
  at SEW=16 (f16 to 32-bit integer) and `vfncvt.x[u].f.w` at SEW=16 (f32 to
  16-bit integer) and their `rtz` forms, truncated whatever `frm` said and
  raised NV only for a NaN (#181). They now round in `frm` (the `rtz` forms
  toward zero) and raise NX for an inexact result and NV for one out of
  range, as the 32- and 64-bit conversions do.

## Releases

- [v2.1.0](versions/V2_1_0_CHANGELOG.md) — 2026-10-08 (device loads, FENCE.I coherence, oldest-only issue timing, `Config(isa=...)`)
- [v2.0.1](versions/V2_0_1_CHANGELOG.md) — 2026-10-07 (decode redirects only when the path changes)
- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
