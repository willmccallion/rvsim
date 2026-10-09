# Tests

Rust unit and integration tests live in the crate, under
`crates/rvsim-core/src/tests/`. This directory holds everything that drives the
simulator from Python.

| Path | What it covers | Command |
|------|----------------|---------|
| `python/` | The `rvsim` Python API (config, session, CLI, stats) | `make test-python` |
| `conformance/riscv_tests.py` | riscv-tests ISA suite on every pipeline config | `make riscv-tests` |
| `conformance/vector/` | chipsalliance RVV tests, cosimulated against spike | `make vector-test` |
| `conformance/vector_tests.py` | The RVV cosim suite on every pipeline config | `make vector-test-multi` |
| `conformance/multicore_tests.py` | Multi-hart programs from `software/bin/multicore` at 2 and 4 harts | `.venv/bin/python tests/conformance/multicore_tests.py` |
| `conformance/spike_compare.py` | riscv-tests commit traces diffed against spike | `.venv/bin/python tests/conformance/spike_compare.py --help` |
| `conformance/lockstep.py` | riscv-tests, programs and benchmarks replayed on spike in lockstep, every pipeline config | `make lockstep`, or `make lockstep-smoke` |
| `run_all.py` | Every conformance suite on every pipeline config | `make test-all`, or `make conformance-smoke` for the smoke subsets |

The pipeline configs the conformance runners sweep are defined in
`conformance/configs/pipelines.py`, along with the smaller sets the smoke run
uses.

## What CI runs

`make test-all-smoke` is the gate a pull request must pass: the Rust and
Python tests, then `make conformance-smoke`. That runs every riscv-test on
the `SMOKE_PIPELINES` configs (both backends, no caches, an L3, every
prefetcher, DDR5, capped paging, a coherent SMP fabric and the calibrated
presets), a sample of every vector instruction class against spike on an
in-order and an out-of-order config, and the multi-hart programs at 2 harts
on every multicore config and at 4 harts on four of them. It takes about
three minutes on four cores. spike, riscv-tests and the vector generator are
pinned to fixed commits in the `Makefile`. Each test runs in a subprocess
(`conformance/_worker.py`), so a hang or crash in one test cannot take down
the run.

## Lockstep against spike

`make lockstep` (and `make test-all`, but not the smoke run CI uses) runs
every riscv-test, bundled program and benchmark on every single-hart pipeline
config with the commit log open, and
`tools/lockstep/spike_lockstep` replays that log on spike one instruction at a
time. It stops at the first instruction whose PC, privilege mode, destination
register, CSR write, FP flags or memory access differs from spike's, and
prints the rvsim log lines leading up to it. Because rvsim's pipeline computes
every value itself, a forwarding, squash or ordering bug shows up here as a
wrong value even though no test checks that value.

Values the ISA leaves to the implementation are taken from rvsim's log
rather than compared: counter, timer and ID CSR reads, device reads,
interrupt arrival, WFI wake-up, a reservation rvsim lost, accesses to
`tcontrol` (which spike lacks), a `misa` write (rvsim's `misa` is
read-only, spike's is not), and `medeleg`'s misaligned-fetch bit, which the
two models choose differently. The driver reports how
many of each it took. Vector register and vector memory effects are not
compared yet.

Test binaries, spike and results are built into `tests/builds/`, which is
ignored by git. `make clean-tests` removes it.
