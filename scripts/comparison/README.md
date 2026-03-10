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

Both simulators use a P550-equivalent config:
- 3-wide OOO, ROB=72, IQ=32
- 32KB L1i/L1d, 256KB L2
- Tournament branch predictor

Edit `config.py` in each script to change the machine model.

## Notes

- gem5 must be built for RISCV: `gem5/build/RISCV/gem5.opt`
- rvsim must be installed: `pip install -e .` or `make build`
- Results are saved to `results/` so you can run the simulators separately
