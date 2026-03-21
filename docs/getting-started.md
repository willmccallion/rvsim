# Getting Started

## Installation

### From PyPI (recommended)

```bash
pip install rvsim
```

Requires Python 3.10+. Pre-built wheels are available for Linux x86_64. The package includes the compiled Rust simulator core — no Rust toolchain needed.

### From source

If you want to modify the simulator itself:

```bash
git clone https://github.com/willmccallion/rvsim
cd rvsim
python3 -m venv .venv && source .venv/bin/activate
pip install maturin
maturin develop --release
```

This builds the Rust core with optimizations and installs it as an editable Python package.

### Building example programs

The included RISC-V programs require a cross-compiler:

```bash
# Install riscv64-unknown-elf-gcc (package name varies by OS)
make -C software
```

This builds the libc, benchmark programs, and test binaries into `software/bin/`.

## Your First Simulation

### Using the Python API

```python
from rvsim import Config, Environment

result = Environment(
    binary="software/bin/programs/qsort.elf",
    config=Config(width=4),
).run()

print(f"Exit code: {result.exit_code}")
print(f"Cycles: {result.stats['cycles']:,}")
print(f"IPC: {result.stats['ipc']:.4f}")
print(f"Instructions: {result.stats['instructions_retired']:,}")
```

### Using the CLI

rvsim can run Python scripts directly:

```bash
rvsim scripts/analysis/branch_predict.py
```

Or run a binary with default settings:

```bash
rvsim -f software/bin/programs/qsort.elf
```

## Understanding the Output

### Stats

Every simulation produces a `Stats` object (a dict subclass) with detailed microarchitectural counters. Use `query()` to filter:

```python
# All cache-related stats
result.stats.query("cache")

# IPC and branch stats
result.stats.query("ipc|branch")

# Everything with "miss" in the name
result.stats.query("miss")
```

### Key metrics

| Metric | What it means |
|--------|---------------|
| `cycles` | Total simulated clock cycles |
| `instructions_retired` | Instructions that completed (committed) |
| `ipc` | Instructions per cycle = instructions_retired / cycles |
| `dcache_misses` | L1 data cache misses |
| `branch_accuracy_pct` | Percentage of branches predicted correctly |
| `stalls_data` | Cycles stalled waiting for data (RAW hazards) |
| `stalls_mem` | Cycles stalled on memory (in-order backend) |
| `stalls_control` | Cycles lost to branch misprediction recovery |

## Comparing Configurations

The simplest way to compare two configurations is `Stats.tabulate()`:

```python
from rvsim import Config, BranchPredictor, Environment, Stats

rows = {}
for name, bp in [
    ("Static", BranchPredictor.Static()),
    ("GShare", BranchPredictor.GShare()),
    ("TAGE", BranchPredictor.TAGE()),
]:
    r = Environment(
        "software/bin/programs/maze.elf",
        Config(branch_predictor=bp),
    ).run()
    rows[name] = r.stats.query("ipc|branch_accuracy|mispredictions")

print(Stats.tabulate(rows, title="Branch Predictor Comparison"))
```

## Parallel Sweeps

For larger experiments, `Sweep` distributes all (binary, config) combinations across CPU cores:

```python
from rvsim import Sweep, Config, Cache

results = Sweep(
    binaries=[
        "software/bin/programs/qsort.elf",
        "software/bin/programs/mandelbrot.elf",
        "software/bin/programs/maze.elf",
    ],
    configs={
        f"L1={s}": Config(
            l1d=Cache(s, ways=8, mshr_count=8),
            uart_quiet=True,
        )
        for s in ["8KB", "16KB", "32KB", "64KB"]
    },
).run(parallel=True)

results.compare(
    metrics=["ipc", "dcache_misses"],
    baseline="L1=8KB",
    col_header="L1D Size",
)
```

This runs all 12 combinations in parallel and prints a comparison table with speedup ratios relative to the baseline.

## Low-Level Control

For fine-grained control, use `Simulator` to build a `Cpu` object and tick it manually:

```python
from rvsim import Simulator, Config, reg, csr

cpu = Simulator().config(Config(width=4)).binary("software/bin/programs/qsort.elf").build()

# Tick 1000 cycles
for _ in range(1000):
    cpu.tick()

# Run until a specific PC
cpu.run_until(pc=0x80001234)

# Run until user-mode code
cpu.run_until(privilege="U")

# Inspect state
print(f"PC: {cpu.pc:#x}")
print(f"a0: {cpu.regs[reg.A0]:#x}")
print(f"sp: {cpu.regs[reg.SP]:#x}")
print(f"mstatus: {cpu.csrs[csr.MSTATUS]:#x}")
print(f"mem[0x80001000]: {cpu.mem64[0x80001000]:#x}")

# Pipeline visualization
cpu.pipeline_snapshot().visualize()

# Checkpoint and restore
cpu.save("checkpoint.bin")
cpu.restore("checkpoint.bin")
```

## Next Steps

- [Configuration Reference](configuration.md) — learn how to configure every aspect of the simulated machine
- [API Reference](api.md) — complete reference for all Python classes
- [Architecture](architecture/pipeline.md) — understand how the pipeline works internally
- [Analysis Scripts](examples/analysis-scripts.md) — run the included design-space exploration scripts
