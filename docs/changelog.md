# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- Multi-core systems: `Config(hart_count=N)` builds N cores with private
  caches, per-hart CLINT/PLIC contexts and device-tree entries; bare-metal
  programs start every hart with its id in `a0`.
- Coherence fabric: MESI states in the private caches, a home agent
  (`HomeAgent.SnoopFilter` or `HomeAgent.Broadcast`) at the LLC and an
  interconnect (`Interconnect.Crossbar`, `Ring`, `Mesh`, `Torus`,
  `Hypercube`), configured with `Config(coherence=Coherence(...))`;
  reported under `coherence.*`.
- Non-blocking caches: MSHRs with coalescing, a writeback buffer, real
  prefetch fetches and honoured inclusion policies; dirty lines reach DRAM.
- `cpu.harts[i]` exposes every hart's `pc`, `privilege`, `regs` and `csrs`;
  `rvsim prog.elf --harts N` on the command line.
- Statistics are rooted at `core<N>` and `hart<N>`, with `system.*` sums.

## Releases

- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
