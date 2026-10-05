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

Test binaries, spike and results are built into `tests/builds/`, which is
ignored by git. `make clean-tests` removes it.
