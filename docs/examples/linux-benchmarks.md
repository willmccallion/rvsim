# Linux Benchmarks

The root filesystem built by `make linux` carries a benchmark suite:
CoreMark, Dhrystone, Whetstone, STREAM, mbw, lmbench's `lat_mem_rd` and
stress-ng. Two tools run it inside Linux on a simulated machine. Both boot
to the login shell once, cache that boot, and measure each benchmark as
its own statistics region on the configuration under study (see
[Measuring a region](../architecture/stats.md#measuring-a-region)).

## `rvsim bench`

```bash
rvsim bench                                  # every benchmark, fast preset, 8 harts
rvsim bench coremark dhrystone --harts 1     # two of them on one hart
rvsim bench --config my_core.py --warm       # your core, each benchmark warmed first
rvsim bench --memory dram --interconnect ring --json out.json
rvsim bench --list                           # the benchmarks and their commands
```

| Option | Meaning |
|--------|---------|
| `NAME ...` | Benchmarks to run: `coremark`, `dhrystone`, `whetstone`, `stream`, `mbw`, `lat_mem_rd`, `stress-ng` (default: all) |
| `--preset NAME` | Core configuration placed in the Linux system: `basic`, `fast` (default), `cortex_a72`, `m1` or `p550` |
| `--config FILE` | A Python file whose `config` variable, `get_config()` or function named after the file gives the core to use instead |
| `--harts N` | Harts (default 8) |
| `--memory ddr5\|dram` | JEDEC DDR5 or the row-buffer DRAM model |
| `--interconnect NAME` | `crossbar`, `ring`, `mesh` (default), `torus` or `hypercube` |
| `--warm` | Run each benchmark once unmeasured before measuring it |
| `--json FILE` | Write every region's statistics and console output |
| `--no-cache` | Boot even if a cached boot exists |
| `--echo` | Show the guest console |

It prints, per benchmark, the cycles, instructions and IPC with branch,
L1D, L2 and LLC misses per thousand instructions.

## Scores per MHz

`tools/diag/linux_bench.py` boots a core preset to the login shell from
the cached checkpoint, runs them through the guest shell, and reports
each one's score from the cycles it took, in the per-MHz figures hardware
is quoted in. The first run on a preset boots Linux (minutes); later runs
start from the cache.

```bash
python tools/diag/linux_bench.py                        # p550, one hart
python tools/diag/linux_bench.py --preset cortex_a72 --harts 2
python tools/diag/linux_bench.py --only coremark --json out.json
```

The scores come from the simulator's cycle count, not the guest's clock:
`DMIPS/MHz = runs / Mcycles / 1757` and `CoreMark/MHz = iterations /
Mcycles`. The runs are short (300,000 Dhrystone runs, 200 CoreMark
iterations), so they include the programs' start-up; the guest's own
timer is too coarse to time them.

## Scores against hardware

Measured 2026-10-04, one hart, from the Linux boot. The hardware columns
are published single-core figures; Dhrystone varies a lot with the
compiler, so Arm's quoted figure and a measured Raspberry Pi 4 are both
given.

| Preset | Benchmark | rvsim | Hardware | Source |
|---|---|---:|---:|---|
| `cortex_a72()` | CoreMark/MHz | 5.19 | 5.50 | Raspberry Pi 4 at 1.5 GHz, single thread |
| `cortex_a72()` | DMIPS/MHz | 3.94 | 3.77 measured, 4.72 quoted | Raspberry Pi 4; Arm's Cortex-A72 figure |
| `p550()` | CoreMark/MHz | 4.59 | not published | |
| `p550()` | DMIPS/MHz | 3.22 | not published | |

SiFive publishes only SPECint2006 for the P550 (8.65 per GHz); no public
CoreMark or Dhrystone figure for the EIC7700X was found. Its preset is
calibrated on the measured microarchitecture instead, below.

## Cache latencies

`tools/diag/latency_probe.py` runs a dependent pointer chase over
footprints that fit each level and reports cycles per load, the load-to-use
latency hardware reviews measure the same way. Both presets reproduce the
measurements the presets were built from (Chips and Cheese on the HiFive
Premier P550 and on a Graviton Cortex-A72).

| Preset | Footprint | rvsim cycles/load | Hardware | Level |
|---|---|---:|---:|---|
| `p550()` | 8 KB | 3.0 | 3 | L1D |
| `p550()` | 128 KB | 13.1 | 13 | L2 |
| `p550()` | 2 MB | 39.0 | ~38 | L3 |
| `p550()` | 16 MB | 267 | 272 (194 ns at 1.4 GHz) | memory |
| `cortex_a72()` | 8 KB | 4.0 | 4 | L1D |
| `cortex_a72()` | 128 KB | 20.3 | 21 | L2 |
| `cortex_a72()` | 16 MB | 242 | 243 (162 ns at 1.5 GHz) | memory |

```bash
python tools/diag/latency_probe.py --preset p550
python tools/diag/latency_probe.py --preset cortex_a72 --footprints 16K,512K,8M,32M
```

The probe needs `riscv64-elf-gcc` on `PATH`, as the repo's nix shell
provides. A 2 MB footprint on the A72 preset mixes L2 hits and memory, so
it is left out of the table.

Sources: Chips and Cheese, "Inside SiFive's P550 Microarchitecture"
(January 2025) and "ARM's Cortex A72: aarch64 for the Masses" (November
2023); the SiFive Performance P550 data sheet (2022); CoreMark and
Dhrystone results for the Raspberry Pi 4 as collected at
kreier.github.io/benchmark and on the Raspberry Pi forums.
