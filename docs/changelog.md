# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- ROB tags wrap after 2^32 allocations, but the out-of-order issue
  queue's select, its stall attribution and snapshot, and the memory1
  and memory2 stages ordered them by their raw numbers. For one window
  after the wrap, younger instructions issued and reached memory before
  older ones (#151). They are now ordered by age, and a tag's number is
  private to the ROB, so it can no longer be used to order tags.
- A cache's `mshr_count`, `write_buffers` and `targets_per_mshr` of 0
  meant the default (8, 8, 20) from Python's `Cache` but 1, a blocking
  cache, in a config dict or a Rust `CacheConfig` (#147). No cache can
  have zero of any of them, so 0 is now refused everywhere: `Cache`
  raises `ValueError`, and a config dict holding 0 fails to load. `Cache`
  takes `None` for the default. **Breaking:** code passing 0 for the
  default must pass `None` or leave the argument out.
- A load from a device took its value from an older store to the same
  bytes still in the store buffer instead of reading the device (#146):
  reading the UART's receive register right after writing its transmit
  register, which share an address, returned the transmitted byte unless
  a FENCE separated them. A device load now waits for older stores to its
  bytes to be written and then reads the device.

## Releases

- [v2.0.1](versions/V2_0_1_CHANGELOG.md) — 2026-10-07 (decode redirects only when the path changes)
- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
