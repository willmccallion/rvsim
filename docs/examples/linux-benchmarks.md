# Linux Benchmarks

The rootfs built by `make linux` carries Dhrystone and CoreMark.
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

## Recorded scores

Measured 2026-10-04 on the `p550()` preset, one hart, from the Linux boot
at commit `2047598`.

| Benchmark | Score | Cycles | Instructions | IPC |
|---|---:|---:|---:|---:|
| Dhrystone | 3.18 DMIPS/MHz | 53,694,257 | 86,904,967 | 1.62 |
| CoreMark | 4.34 CoreMark/MHz | 46,088,015 | 73,459,412 | 1.59 |

Published figures for the EIC7700X's P550 and for the Cortex-A72, and the
`cortex_a72()` preset's rows, are the subject of #84; the presets are to
be calibrated against them.
