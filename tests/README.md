# Tests

Rust unit and integration tests live next to the crate in
`crates/rvsim-core/tests/`. This directory holds everything that drives the
simulator from Python.

| Path | What it covers | Command |
|------|----------------|---------|
| `python/` | The `rvsim` Python API (config, session, CLI, stats) | `make test-python` |
| `conformance/riscv_tests.py` | riscv-tests ISA suite on every pipeline config | `make riscv-tests` |
| `conformance/riscof_tests.py` | riscv-arch-test through riscof on every pipeline config | `make arch-test-multi` |
| `conformance/vector/` | chipsalliance RVV tests, cosimulated against spike | `make vector-test` |
| `conformance/vector_tests.py` | The RVV cosim suite on every pipeline config | `make vector-test-multi` |
| `conformance/multicore_tests.py` | Multi-hart programs from `software/bin/multicore` | `.venv/bin/python tests/conformance/multicore_tests.py` |
| `conformance/spike_compare.py` | riscv-tests commit traces diffed against spike | `.venv/bin/python tests/conformance/spike_compare.py --help` |
| `run_all.py` | Every conformance suite on every pipeline config | `make test-all` (or `make test-all-smoke`) |

The pipeline configs the conformance runners sweep are defined in
`conformance/configs/pipelines.py`. Each test runs in a subprocess
(`conformance/_worker.py`), so a hang or crash in one test cannot take down
the run.

Test binaries, spike and results are built into `tests/builds/`, which is
ignored by git. `make clean-tests` removes it.
