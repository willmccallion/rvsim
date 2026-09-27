# rvsim vs gem5 Comparison

Runs the same binaries through both simulators and compares IPC, cycle counts, and branch accuracy.

## Usage

**Step 1 — run rvsim:**
```bash
python scripts/comparison/run_rvsim.py [binary1.elf binary2.elf ...]
# defaults to qsort, maze, mandelbrot, merge_sort
# outputs: scripts/comparison/results/rvsim.json
```

**Step 2 — run gem5:**
```bash
gem5.opt scripts/comparison/run_gem5.py [binary1.elf binary2.elf ...]
# outputs: scripts/comparison/results/gem5.json
```

**Step 3 — compare:**
```bash
python scripts/comparison/compare.py
# reads both JSONs and prints a side-by-side table
```

## Config

Both simulators use a P550-like config:
- 3-wide OOO, ROB=72, IQ=32
- 32KB L1i/L1d, 256KB L2
- Tournament branch predictor

`run_rvsim.py` (`p550_config`) mirrors the machine `gem5_single.py` builds,
taking every value it leaves unset from gem5's defaults: the O3 FU pool
latencies, the stdlib L1/L2 caches (tag and data 1/1 and 10/10, accessed in
parallel; 16/16/20 MSHRs; 16-way L2), the direct-mapped 4096-entry
`SimpleBTB`, the 16-entry RAS and `TournamentBP`'s table sizes.

What no rvsim setting can express:

- gem5 pools some operations that rvsim gives separate units. Branches share
  the three `IntALU`s; integer multiply and divide share one `IntMultDiv`; FP
  multiply, FMA, divide and square root share two `FP_MultDiv` units, where
  square root takes 24 cycles and divide 12 (rvsim uses 12 for both).
  Sign injection and `fclass` are `FloatMisc` there (3 cycles on
  `FP_MultDiv`); rvsim runs them on the FP adder.
- gem5 puts an `L2XBar` between the L1s and the L2, adding a cycle each way.
- gem5 runs in syscall-emulation mode on DDR3-1600; rvsim uses its
  fixed-latency controller at the same 12.8 GiB/s.
- Stage-to-stage delays differ: gem5's O3 has two cycles from rename to IEW.

`make compare-gem5` runs rvsim, runs gem5 when `gem5.opt` is on `PATH` (or
`GEM5_BIN` is set), and prints the table; without gem5 it compares with the
stored `results/gem5.json`.

## Notes

- gem5 must be built for RISCV: `gem5/build/RISCV/gem5.opt`
- rvsim must be installed: `pip install -e .` or `make build`
- Results are saved to `results/` so you can run the simulators separately
