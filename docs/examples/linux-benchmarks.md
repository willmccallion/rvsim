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
rvsim bench --memory Dram --interconnect Ring --json out.json
rvsim bench --list                           # the benchmarks and their commands
```

| Option | Meaning |
|--------|---------|
| `NAME ...` | Benchmarks to run: `coremark`, `dhrystone`, `whetstone`, `stream`, `mbw`, `lat_mem_rd`, `stress-ng` (default: all) |
| `--preset NAME` | Core configuration placed in the Linux system: `basic`, `fast` (default), `rocket` or `boom` |
| `--config FILE` | A Python file whose `config` variable, `get_config()` or function named after the file gives the core to use instead |
| `--harts N` | Harts (default 8) |
| `--memory Ddr5\|Dram` | JEDEC DDR5 or the row-buffer DRAM model |
| `--interconnect NAME` | `Crossbar`, `Ring`, `Mesh` (default), `Torus` or `Hypercube` |
| `--warm` | Run each benchmark once unmeasured before measuring it |
| `--json FILE` | Write every region's statistics and console output |
| `--no-cache` | Boot even if a cached boot exists |
| `--echo` | Show the guest console |

It prints, per benchmark, the cycles, instructions and IPC with branch,
L1D, L2 and LLC misses per thousand instructions.

## Scores per MHz

`tools/diag/linux_bench.py` boots a core, one of the configurations
calibrated to measured hardware (`tools/diag/calibration.py`) or the
`fast` preset, to the login shell from the cached checkpoint, runs them through the guest shell, and reports
each one's score from the cycles it took, in the per-MHz figures hardware
is quoted in. The first run on a core boots Linux (minutes); later runs
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

Measured 2026-11-24, one hart, from the Linux boot, with the calibrated
configurations' load/store-unit prefetchers. The hardware columns
are published single-core figures; Dhrystone varies a lot with the
compiler, so Arm's quoted figure and a measured Raspberry Pi 4 are both
given.

| Calibrated configuration | Benchmark | rvsim | Hardware | Source |
|---|---|---:|---:|---|
| `cortex_a72` | CoreMark/MHz | 5.12 | 5.50 | Raspberry Pi 4 at 1.5 GHz, single thread |
| `cortex_a72` | DMIPS/MHz | 3.89 | 3.77 measured, 4.72 quoted | Raspberry Pi 4; Arm's Cortex-A72 figure |
| `p550` | CoreMark/MHz | 4.72 | not published | |
| `p550` | DMIPS/MHz | 3.29 | not published | |

SiFive publishes only SPECint2006 for the P550 (8.65 per GHz); no public
CoreMark or Dhrystone figure for the EIC7700X was found. Its configuration
is calibrated on the measured microarchitecture instead, below.

## Cache latencies

`tools/diag/latency_probe.py` runs a dependent pointer chase over
footprints that fit each level and reports cycles per load, the load-to-use
latency hardware reviews measure the same way. It runs each footprint
twice: bare-metal, with no address translation, which compares with a
measurement on huge pages; and in SE mode under 4 KB pages, where TLB
misses and page walks add their cost. Measured 2026-11-24.

The P550 figures are Chips and Cheese's on the HiFive Premier P550, which
do not say which page size they used; the configuration is fitted to the
bare column.

| `p550` footprint | Bare | 4 KB pages | Hardware | Level |
|---|---:|---:|---:|---|
| 8 KB | 3.0 | 3.0 | 3 | L1D |
| 128 KB | 13.1 | 13.3 | 13 | L2 |
| 2 MB | 39.0 | 47.6 | ~38 | L3 |
| 16 MB | 268 | 282 | 272 (194 ns at 1.4 GHz) | memory |

The Cortex-A72 figures are tinymembench's on a Raspberry Pi 4, which
reports both page sizes, with its 4-cycle L1 hit (2.7 ns at 1.5 GHz)
added back, since tinymembench reports only the time beyond it.

| `cortex_a72` footprint | Bare | Pi 4, huge pages | 4 KB pages | Pi 4, 4 KB pages | Level |
|---|---:|---:|---:|---:|---|
| 512 KB | 20.1 | 20.1 (13.4 ns) | 25.4 | 25.3 (16.9 ns) | L2 |
| 16 MB | 206 | 203 (135.0 ns) | 219 | 233 (155.6 ns) | memory |

Under 4 KB pages at 16 MB, page walks cost the configuration 13 cycles a
load and the Pi 4 about 30; the walker's own cache accesses are the
likely difference, not yet investigated.

```bash
python tools/diag/latency_probe.py --preset p550
python tools/diag/latency_probe.py --preset cortex_a72 --footprints 16K,512K,8M,32M
```

The probe needs `riscv64-elf-gcc` and `riscv64-linux-gnu-gcc` on `PATH`,
as the repo's nix shell provides. A 2 MB footprint on the A72 configuration
mixes L2 hits and memory, so it is left out of the table.

Sources: Chips and Cheese, "Inside SiFive's P550 Microarchitecture"
(January 2025) and "ARM's Cortex A72: aarch64 for the Masses" (November
2023); tinymembench's "Raspberry Pi 4 (BCM2711)" results
(github.com/ssvb/tinymembench wiki, January 2024); the SiFive Performance
P550 data sheet (2022); CoreMark and
Dhrystone results for the Raspberry Pi 4 as collected at
kreier.github.io/benchmark and on the Raspberry Pi forums.
