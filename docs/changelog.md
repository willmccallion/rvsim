# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- `Simulator.open_commit_log` writes what a reference model needs to replay
  the run: the architectural state the log starts from, each retired
  instruction's privilege mode, destination register, the CSR it wrote (as
  read back), the FP flags it raised and its load or store (virtual and
  physical address, size and value), and each trap taken. An FP destination
  was logged as an `x` register. Vector register and vector memory effects
  are not logged yet.

## Releases

- [v2.1.0](versions/V2_1_0_CHANGELOG.md) — 2026-10-08 (device loads, FENCE.I coherence, oldest-only issue timing, `Config(isa=...)`)
- [v2.0.1](versions/V2_0_1_CHANGELOG.md) — 2026-10-07 (decode redirects only when the path changes)
- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
