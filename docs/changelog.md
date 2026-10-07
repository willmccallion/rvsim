# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- Decode refetched the path fetch had already taken whenever it found a
  branch the BTB missed, predicted it not taken, and fetch had predicted a
  younger branch: it undid the younger predictions, which were made
  without the branch in the histories, and redirected fetch to remake
  them. A never-taken branch, which never enters the BTB, paid this every
  loop iteration, so how often it happened depended on front-end timing
  and moved IPC with unrelated parameters (#143). Decode now redoes those
  predictions in program order as it reaches them and redirects only when
  the path changes. `bp.decode_redirects` falls by up to 99% and the
  affected programs take up to 10% fewer cycles; mispredictions are
  unchanged.

## Releases

- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
