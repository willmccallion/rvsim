# Benchmark Configurations

The `scripts/benchmarks/` directory contains microarchitecture-specific configurations modeled after real hardware. These are used by the analysis scripts (particularly `top_down.py`) and can serve as starting points for your own experiments.

## Available Configurations

### SiFive Performance P550

Based on published microarchitecture analysis (Chips and Cheese, SiFive specs).

```python
from scripts.benchmarks.p550.config import p550_config
config = p550_config()
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Width | 3 | Triple-issue |
| Backend | OutOfOrder | 72-entry ROB, 32-entry IQ |
| Branch Predictor | Tournament | 9.1 KiB budget |
| L1D | 32KB, 8-way, 3cy | 8 MSHRs, stride prefetch |
| L2 | 256KB, 8-way, 10cy | 16 MSHRs |
| L3 | 4MB, 16-way, 30cy | 32 MSHRs |
| Memory | DRAM | t_cas=14, row_miss=120 |

### ARM Cortex-A72

Based on publicly documented microarchitecture.

```python
from scripts.benchmarks.cortex_a72.config import cortex_a72_config
config = cortex_a72_config()
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Width | 3 | Triple-issue |
| Backend | OutOfOrder | 128-entry ROB, 60-entry IQ |
| Branch Predictor | TAGE | 4 banks, 2048-entry tables |
| L1I | 48KB, 3-way | NextLine prefetch |
| L1D | 32KB, 2-way | 8 MSHRs, stride prefetch |
| L2 | 1MB, 16-way, 12cy | 16 MSHRs |

### Apple M1

Modeled after Apple's Firestorm (performance) core.

```python
from scripts.benchmarks.m1.config import m1_config
config = m1_config()
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Width | 4 | Quad-issue |
| Branch Predictor | TAGE | 4 banks, 4096-entry tables |
| L1I | 128KB, 8-way | NextLine prefetch (degree=2) |
| L1D | 128KB, 8-way | 12 MSHRs, stride prefetch (degree=2) |
| L2 | 4MB, 16-way, 12cy | 32 MSHRs |

## Using Benchmark Configs

### With Environment

```python
from scripts.benchmarks.p550.config import p550_config
from rvsim import Environment

result = Environment(
    binary="software/bin/programs/qsort.elf",
    config=p550_config(),
).run()

print(result.stats.query("ipc|stall|miss"))
```

### With Sweep

```python
from scripts.benchmarks.p550.config import p550_config
from scripts.benchmarks.cortex_a72.config import cortex_a72_config
from scripts.benchmarks.m1.config import m1_config
from rvsim import Sweep

results = Sweep(
    binaries=["software/bin/programs/qsort.elf"],
    configs={
        "P550": p550_config(),
        "A72": cortex_a72_config(),
        "M1": m1_config(),
    },
).run(parallel=True)

results.compare(metrics=["ipc", "cycles", "dcache_misses", "branch_accuracy_pct"])
```

### Deriving Variants

Use `replace()` to create variants of a benchmark config:

```python
base = p550_config()
wide = base.replace(width=4)
big_cache = base.replace(l1d=Cache("64KB", ways=8, latency=3, mshr_count=8))
```

## Creating Your Own Config

Follow the pattern in the existing configs. A config file should:

1. Define a function that returns a `Config` object
2. Assign it to a module-level `config` variable (for `Simulator.config("path/to/config.py")` support)

```python
"""My custom machine config."""
from rvsim import Config, Cache, Backend, BranchPredictor, Fu

def my_config():
    return Config(
        width=2,
        backend=Backend.OutOfOrder(
            rob_size=64,
            issue_queue_size=24,
            fu_config=Fu([
                Fu.IntAlu(count=2, latency=1),
                Fu.IntMul(count=1, latency=3),
                Fu.Branch(count=1, latency=1),
                Fu.Mem(count=1, latency=1),
            ]),
        ),
        branch_predictor=BranchPredictor.GShare(),
        l1d=Cache("16KB", ways=4, latency=1, mshr_count=4),
        l2=Cache("128KB", ways=8, latency=8),
    )

config = my_config
```
