# rvsim vs gem5 Comparison

Runs the same programs on rvsim and on gem5's O3 CPU, each configured as the
same machine, and compares cycles, branch mispredictions and cache misses.

## Usage

```bash
bash scripts/comparison/programs/build.sh     # -> testing/builds/compare-programs/*.elf
python scripts/comparison/run_rvsim.py [variant ...]
GEM5_BIN=path/to/gem5.opt python scripts/comparison/run_gem5.py [variant ...]
python scripts/comparison/compare.py
```

`make compare-gem5` does all four; without `GEM5_BIN` it compares with the
stored `results/gem5.json`.

## Programs

`programs/` holds small C programs that do no I/O, so one ELF runs unchanged
on rvsim and in gem5's syscall emulation. Each isolates one behaviour: branch
patterns, calls and indirect calls, memory access patterns, ALU and FP
latency chains, store-to-load forwarding, and vector unit-stride, strided,
indexed and reduction code. They are built for
`rv64gcv_zba_zbb_zbc_zbs_zbkb_zbkx_zfh`, every extension both simulators
implement (gem5 lacks the vector crypto extensions).

## Machine

`variants.py` describes each machine as plain data, and both
`run_rvsim.py` and `gem5_single.py` build their simulator from it. The base
is P550-like: 3-wide O3, ROB 72, IQ 32, LQ 24, SQ 16, 32 KiB L1s, 256 KiB L2,
tournament predictor, VLEN 256, fixed-latency memory at 12.8 GiB/s. Each
other variant changes one thing: the predictor (TAGE, a small tournament),
the cache sizes, the L1D's MSHRs, or a tagged L1D prefetcher.

What no rvsim setting can express:

- gem5 pools some operations that rvsim gives separate units. Branches share
  the three `IntALU`s; integer multiply and divide share one `IntMultDiv`; FP
  multiply, FMA, divide and square root share two `FP_MultDiv` units, where
  square root takes 24 cycles and divide 12 (rvsim uses 12 for both). All
  vector arithmetic shares gem5's `SIMD_Unit`s; rvsim gets that many of each
  vector unit.
- gem5 splits a vector instruction into one micro-op per register of its
  group; rvsim models lanes, set here to one register per cycle.
- Stage-to-stage delays differ: gem5's O3 has two cycles from rename to IEW.

## Reading the stats

- Loads, stores and L1D accesses count differently for vector code: gem5
  counts micro-ops and cache packets, rvsim instructions and its own accesses.
- gem5 counts `vset{i}vl{i}` as control instructions.
- L1I accesses differ in granularity; only misses are compared.
