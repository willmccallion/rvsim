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
- An instruction that must be the oldest (a CSR access, ECALL, EBREAK,
  xRET, WFI, FENCE.I, SFENCE.VMA, an AMO or store-conditional, and on the
  in-order backend a vector instruction) issued in the same cycle the
  instruction ahead of it retired, as commit ran before issue in the same
  cycle (#150). It now checks the ROB head as the cycle began, so it
  issues at the earliest the cycle after: one cycle more per such
  instruction on both backends. A device read, which memory1 holds until
  it is the oldest instruction, waits for the same latched head.
- FENCE.I cost the same however many lines the L1D held dirty, as nothing
  made the instruction side see them (#149). How hardware pays depends on
  the hierarchy, and rvsim now follows it. With an L2, an instruction
  fetch for a line the L1D may hold writable probes the L1D through the
  L2 first, which writes it back if it is dirty, as SiFive's and Rocket's
  coherent L2s do (`l2.fetch_probes`, `l2.fetch_probes_dirty`). Without
  one, FENCE.I flushes the L1D, as Rocket does with no coherence manager:
  the L1D walks every line, one a cycle, writes back the dirty ones
  through its writeback buffer and invalidates them all before FENCE.I
  retires, skipping the walk when it has fetched nothing since its last
  flush (`l1d.flushes`, `l1d.flushed_lines`).
- Fetch and decode treated a halfword whose low bits are not `11` as a
  16-bit instruction even when `misa` had no C, so a hart without C ran
  compressed encodings instead of raising illegal-instruction, and its
  wrong-path fetch stepped 2 bytes at a time through zero padding where
  hardware without C steps 4 (#156). Fetch now sizes instructions from
  the current `misa`, as branch targets already did.
  `Config(isa="RV64IM")` sets the hart's ISA from Python.
- On the in-order backend, fetch started at the target of a redirect
  commit took (a trap, xRET, FENCE.I, SFENCE.VMA, a re-execute) in the
  same cycle, one cycle earlier than on the out-of-order backend and in
  hardware, where the redirect is registered first (#157). It now starts
  the cycle after; redirects from execute already waited
  `redirect_latency`.

## Releases

- [v2.0.1](versions/V2_0_1_CHANGELOG.md) — 2026-10-07 (decode redirects only when the path changes)
- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
