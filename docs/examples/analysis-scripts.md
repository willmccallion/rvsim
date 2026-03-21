# Analysis Scripts

The `scripts/analysis/` directory contains ready-to-run design-space exploration scripts. Each script uses the `Sweep` API to run parallel experiments and print comparison tables.

## Running Scripts

All scripts can be run with the `rvsim` CLI:

```bash
rvsim scripts/analysis/branch_predict.py
rvsim scripts/analysis/cache_sweep.py --sizes 4KB 8KB 16KB 32KB
rvsim scripts/analysis/o3_inorder.py --widths 1 2 4
```

Or directly with Python:

```bash
.venv/bin/python scripts/analysis/branch_predict.py
```

!!! note "Prerequisites"
    The analysis scripts require the example RISC-V programs to be built first:
    ```bash
    make -C software
    ```

---

## Available Scripts

### branch_predict.py

Compares all five branch predictors across multiple workloads.

```bash
rvsim scripts/analysis/branch_predict.py
rvsim scripts/analysis/branch_predict.py --width 4 --programs maze qsort
```

**Metrics:** cycles, IPC, branch accuracy, mispredictions, correct predictions

**Example output:**

```
  ›  branch_accuracy_pct
  predictor        Static   GShare     TAGE  Perceptron  Tournament
  mandelbrot.elf  56.4375  57.3566  97.7840     54.3290     97.2340
  maze.elf        51.6078  56.9724  98.6112     46.0765     80.7168
  qsort.elf       32.5515  60.3781  83.4249     67.1985     69.7478
  merge_sort.elf  53.7159  65.3798  84.9415     66.7236     82.8787
```

### cache_sweep.py

Sweeps L1 data cache size and measures miss rate and IPC impact.

```bash
rvsim scripts/analysis/cache_sweep.py
rvsim scripts/analysis/cache_sweep.py --sizes 1KB 2KB 4KB 8KB 16KB 32KB --ways 8
```

**Metrics:** cycles, IPC, dcache hits, dcache misses

### design_space.py

Multi-dimensional sweep across pipeline width and L1D cache size.

```bash
rvsim scripts/analysis/design_space.py
rvsim scripts/analysis/design_space.py software/bin/programs/maze.elf
```

**Sweep dimensions:** width (1, 2, 4, 8) × L1D size (16KB, 32KB, 64KB, 128KB) = 16 configurations

**Metrics:** IPC, cycles, dcache misses

### o3_inorder.py

Compares the out-of-order and in-order backends at different pipeline widths.

```bash
rvsim scripts/analysis/o3_inorder.py
rvsim scripts/analysis/o3_inorder.py --widths 1 2 4 8
```

**Metrics:** IPC, cycles, instructions retired, data stalls, memory stalls, control stalls

**Example output:**

```
  ›  ipc
  config          inorder_w1   o3_w1  inorder_w4   o3_w4
  qsort.elf           0.3935  0.7622      0.4492  0.9256
  mandelbrot.elf      0.5562  0.9151      0.7060  1.4751
```

### width_scaling.py

Measures how IPC scales with superscalar width using a fixed predictor.

```bash
rvsim scripts/analysis/width_scaling.py
rvsim scripts/analysis/width_scaling.py --bp TAGE --widths 1 2 4 8
```

**Metrics:** cycles, IPC, branch accuracy, mispredictions

### stall_breakdown.py

Breaks down stall cycles into memory, control (misprediction), and data (RAW hazard) categories.

```bash
rvsim scripts/analysis/stall_breakdown.py
```

**Metrics per program:** cycles, IPC, control stalls, data stalls, memory stalls

### top_down.py

Top-down microarchitecture analysis using the standard four-category breakdown:

- **Retiring**: cycles doing useful work
- **Bad Speculation**: cycles wasted on mispredicted paths
- **Backend Bound**: cycles waiting for execution resources or data
- **Frontend Bound**: cycles where the frontend can't deliver instructions

```bash
rvsim scripts/analysis/top_down.py
rvsim scripts/analysis/top_down.py software/bin/programs/maze.elf
```

### inst_mix.py

Instruction class breakdown showing the distribution of ALU, load, store, branch, system, and FP instructions.

```bash
rvsim scripts/analysis/inst_mix.py
```

---

## Writing Your Own Scripts

All scripts follow the same pattern:

```python
from rvsim import Config, Cache, BranchPredictor, Sweep

# Define configurations to compare
configs = {
    "baseline": Config(width=4, uart_quiet=True),
    "big_cache": Config(width=4, l1d=Cache("64KB", ways=8, mshr_count=8), uart_quiet=True),
}

# Define workloads
binaries = [
    "software/bin/programs/qsort.elf",
    "software/bin/programs/maze.elf",
]

# Run and compare
results = Sweep(binaries=binaries, configs=configs).run(parallel=True)
results.compare(
    metrics=["ipc", "cycles", "dcache_misses"],
    baseline="baseline",
)
```

!!! tip "uart_quiet"
    Set `uart_quiet=True` in sweep configs to suppress UART output from the simulated programs. This prevents interleaved output from parallel runs.
