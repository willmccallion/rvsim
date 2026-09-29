# Analysis Scripts

The `examples/analysis/` directory contains ready-to-run design-space exploration scripts. Each script uses the `Sweep` API to run parallel experiments and print comparison tables.

## Running Scripts

All scripts can be run with the `rvsim` CLI:

```bash
rvsim examples/analysis/branch_predict.py
rvsim examples/analysis/cache_sweep.py --sizes 4KB 8KB 16KB 32KB
rvsim examples/analysis/o3_inorder.py --widths 1 2 4
```

Or directly with Python:

```bash
.venv/bin/python examples/analysis/branch_predict.py
```

!!! note "Prerequisites"
    The analysis scripts require the example RISC-V programs to be built first:
    ```bash
    make -C software
    ```

---

## Available Scripts

### branch_predict.py

Compares all six branch predictors across multiple workloads.

```bash
rvsim examples/analysis/branch_predict.py
rvsim examples/analysis/branch_predict.py --width 4 --programs maze qsort
```

**Metrics:** cycles, IPC, committed branch accuracy and mispredictions

**Example output** (`--programs qsort maze`, accuracy table):

```
  ›  core0.bp.committed.accuracy
  predictor  Static  GShare    TAGE  ScLTage  Perceptron  Tournament
  qsort.elf  0.3623  0.8553  0.8749   0.8834      0.8770      0.8654
  maze.elf   0.6566  0.9849  0.9917   0.9923      0.9892      0.9857
  AGGREGATE  0.4042  0.8738  0.8915   0.8989      0.8930      0.8826
```

### cache_sweep.py

Sweeps L1 data cache size and measures miss rate and IPC impact.

```bash
rvsim examples/analysis/cache_sweep.py
rvsim examples/analysis/cache_sweep.py --sizes 1KB 2KB 4KB 8KB 16KB 32KB --ways 8
```

**Metrics:** cycles, IPC, L1D miss rate, L1D misses

### design_space.py

Multi-dimensional sweep across pipeline width and L1D cache size.

```bash
rvsim examples/analysis/design_space.py
rvsim examples/analysis/design_space.py software/bin/programs/maze.elf
```

**Sweep dimensions:** width (1, 2, 4, 8) × L1D size (16KB, 32KB, 64KB, 128KB) = 16 configurations

**Metrics:** IPC, cycles, L1D misses

### o3_inorder.py

Compares the out-of-order and in-order backends at different pipeline widths.

```bash
rvsim examples/analysis/o3_inorder.py
rvsim examples/analysis/o3_inorder.py --widths 1 2 4 8
```

**Metrics:** IPC, cycles, instructions retired, data, control and functional-unit stalls

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
rvsim examples/analysis/width_scaling.py
rvsim examples/analysis/width_scaling.py --bp TAGE --widths 1 2 4 8
```

**Metrics:** cycles, IPC, committed branch accuracy and mispredictions

### stall_breakdown.py

Breaks down stall cycles by cause: control (misprediction recovery), data (waiting for operands), dispatch, functional units, squashes and the rest of `core0.pipeline.stalls.*`.

```bash
rvsim examples/analysis/stall_breakdown.py
```

**Metrics per program:** cycles, IPC, every `core0.pipeline.stalls.*` counter

### top_down.py

Top-down microarchitecture analysis using the standard four-category breakdown:

- **Retiring**: cycles doing useful work
- **Bad Speculation**: cycles wasted on mispredicted paths
- **Backend Bound**: cycles waiting for execution resources or data
- **Frontend Bound**: cycles where the frontend can't deliver instructions

```bash
rvsim examples/analysis/top_down.py
rvsim examples/analysis/top_down.py software/bin/programs/maze.elf
```

### inst_mix.py

Instruction class breakdown showing the distribution of ALU, load, store, branch, system, and FP instructions.

```bash
rvsim examples/analysis/inst_mix.py
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
    metrics=["ipc", "cycles", "core0.cache.l1d.misses"],
    baseline="baseline",
)
```

!!! tip "uart_quiet"
    Set `uart_quiet=True` in sweep configs to suppress UART output from the simulated programs. This prevents interleaved output from parallel runs.
