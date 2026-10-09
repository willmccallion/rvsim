# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

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

## Releases

- [v2.1.0](versions/V2_1_0_CHANGELOG.md) — 2026-10-08 (device loads, FENCE.I coherence, oldest-only issue timing, `Config(isa=...)`)
- [v2.0.1](versions/V2_0_1_CHANGELOG.md) — 2026-10-07 (decode redirects only when the path changes)
- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
