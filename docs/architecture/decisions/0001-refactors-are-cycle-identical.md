# 1. Structural changes are cycle-identical and gated

**Context.** Restructuring the pipeline code while also changing its timing
makes it impossible to tell a refactoring mistake from an intended timing
change.

**Decision.** A structural commit must not change timing. Every such commit
is gated on:

- `scripts/diag/cycle_baseline.py --compare <baseline>`: 216 program and
  configuration pairs must match in cycles, retired instructions and exit
  code;
- `scripts/diag/linux_baseline.py --hart-count 1 --limit 20000000`: the
  retired-instruction count at a fixed cycle budget must match;
- `cargo test`, `cargo clippy --workspace --all-targets`, riscv-tests,
  the vector suite, the multi-core suite and the Python tests.

A commit that is meant to change timing says so, explains the change in
cycles, and records a new baseline for the commits after it.

**Consequences.** Timing changes are isolated in their own commits with
their own regression tests, and the baseline file names the point the
comparison starts from. Baselines live in `testing/builds/results/`, which
is not version-controlled.
