# Error against gem5

How far rvsim's cycle counts are from gem5's O3 CPU on the same programs.
Measured 2026-09-27 at commit `056c862` with `make compare-gem5`.

## Method

Both simulators run the microbenchmarks in `examples/benchmarks/microbenchmarks`
(binaries in `software/bin/benchmarks`) on the P550-like machine that
`scripts/comparison/gem5_single.py` builds: 3-wide, 72-entry ROB, Tournament
predictor, 32 KiB L1s, 256 KiB L2, 1.4 GHz. rvsim uses `p550_config` in
`scripts/comparison/run_rvsim.py`, which copies every gem5 parameter and
default it can express. `scripts/comparison/README.md` lists the ones it
cannot.

Error is `(rvsim cycles - gem5 cycles) / gem5 cycles`, over the whole
program. A positive error means rvsim is slower than gem5.

## Results

| Benchmark | gem5 cycles | rvsim cycles | Error | gem5 IPC | rvsim IPC | gem5 mispredicts | rvsim mispredicts |
|---|---:|---:|---:|---:|---:|---:|---:|
| `alu_int_mul` | 60,522 | 102,553 | +69.4% | 2.15 | 1.27 | 16 | 34 |
| `alu_int_div` | 71,637 | 112,837 | +57.5% | 2.23 | 1.42 | 16 | 34 |
| `alu_fp_add` | 54,756 | 92,891 | +69.6% | 2.56 | 1.51 | 15 | 34 |
| `pipe_load_use` | 30,601 | 42,593 | +39.2% | 2.29 | 1.65 | 16 | 34 |
| `pipe_raw_hazard` | 65,585 | 102,616 | +56.5% | 1.68 | 1.08 | 16 | 34 |
| `bp_always_taken` | 60,523 | 102,528 | +69.4% | 2.15 | 1.27 | 15 | 34 |
| `bp_never_taken` | 60,521 | 92,521 | +52.9% | 1.82 | 1.19 | 15 | 25 |
| `bp_pattern_alt` | 60,636 | 92,860 | +53.1% | 2.39 | 1.57 | 22 | 42 |
| `bp_random` | 197,586 | 272,114 | +37.7% | 1.44 | 1.05 | 4,988 | 5,004 |
| `cache_linear_read` | 714,761 | 1,137,725 | +59.2% | 1.19 | 0.75 | 29 | 39 |
| `cache_strided_read` | 447,923 | 361,047 | -19.4% | 1.08 | 1.34 | 29 | 39 |
| `cache_thrash_assoc` | 371,953 | 295,658 | -20.5% | 1.25 | 1.57 | 29 | 39 |
| `cache_write_heavy` | 260,964 | 273,639 | +4.9% | 1.70 | 1.62 | 4,145 | 4,155 |
| `mem_rand_walk` | 692,367 | 602,435 | -13.0% | 0.76 | 0.88 | 27 | 34 |

Across these 14 programs:

- **Mean absolute error:** 44.5%.
- **Median absolute error:** 53.0%.

## Caveats

- The gem5 numbers are the stored `scripts/comparison/results/gem5.json`,
  recorded 2026-05-03. No gem5 binary is built on this machine, so they were
  not rerun. The binaries were rebuilt since then, but both simulators retire
  the same instruction counts to within 0.4% (a few hundred startup
  instructions), except `mem_rand_walk` at 1.6%.
- `mix_matrix_mul` is left out. gem5 retired 38% more instructions than
  rvsim, so it ran a different build of the program.
- gem5 runs in syscall-emulation mode on DDR3-1600. rvsim runs bare-metal on
  its fixed-latency memory controller.

## Known causes

- **Compute-bound kernels run 40-70% slow.** Not yet root-caused. Compiled at
  -O0, they are chains of loads and stores through the stack. The leading
  suspect is stage timing: gem5's O3 has two cycles from rename to IEW and
  one from issue to execute, which rvsim does not match.
- **Branch predictors still train more slowly than gem5's.** The kernels
  without data-dependent branches mispredict 25-42 times where gem5
  mispredicts 15-29. The predictor now
  follows gem5's `TournamentBP` and its per-branch history, so the gap is
  not yet root-caused.
- **Cache-bound kernels run 13-20% fast.** Not yet root-caused. rvsim has no
  `L2XBar` between the L1s and the L2, and its DRAM model differs; both make
  its misses cheaper.
