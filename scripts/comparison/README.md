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

The machine models are defined in `run_rvsim.py` (`p550_config`) and
`gem5_single.py`. They do not match exactly: gem5 keeps its default
functional-unit latencies (e.g. `FP_ALU` 2 cycles where rvsim uses 5), has
two FP ALUs, runs in syscall-emulation mode on DDR3-1600, and uses the
classic cache hierarchy's default latencies. Compare kernels that are
bound by the same resources on both.

`make compare-gem5` runs rvsim, runs gem5 when `gem5.opt` is on `PATH` (or
`GEM5_BIN` is set), and prints the table; without gem5 it compares with the
stored `results/gem5.json`.

## Notes

- gem5 must be built for RISCV: `gem5/build/RISCV/gem5.opt`
- rvsim must be installed: `pip install -e .` or `make build`
- Results are saved to `results/` so you can run the simulators separately
