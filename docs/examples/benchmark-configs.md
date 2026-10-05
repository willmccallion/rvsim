# Benchmark Configurations

`rvsim.presets` includes configurations modeled after real hardware. The analysis examples use them, and they make good starting points for your own experiments.

## Available Configurations

| Preset | What it is |
|--------|------------|
| `basic()` | `Config()`: a modest 4-wide out-of-order core with 32 KiB L1s and a 256 KiB L2 |
| `fast()` | An Apple M4 P-core class core, the default core of the Linux system |
| `p550()` | SiFive's Performance P550, calibrated to measured hardware |
| `cortex_a72()` | Arm's Cortex-A72 at the Raspberry Pi 4's clock, calibrated to measured hardware |
| `m1()` | A 4-wide core with Apple M1-sized caches |
| `linux(harts, core=...)` | A core placed in the system that boots the bundled Linux image |

### Fast (Apple M4 P-core class)

```python
from rvsim import presets

config = presets.fast()
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Width | 8 | 4.4 GHz |
| Backend | OutOfOrder | 630-entry ROB, 160-entry IQ, 140-entry LQ, 108-entry SQ, 3 load and 2 store ports, 64 branch checkpoints |
| Functional units | 6 ALU, 2 MUL, 4 each of FP add, multiply and FMA, 4 address units | Vector: VLEN 256, 4 lanes, chaining |
| Branch Predictor | 64KB TAGE-SC-L with ITTAGE | 16K-entry 8-way BTB, 48-entry RAS |
| L1I | 192KB, 6-way, PLRU | NextLine prefetch (degree 3), 10 MSHRs |
| L1D | 128KB, 8-way, PLRU | 4-cycle load-to-use, 20 MSHRs, stride prefetch (degree 4) |
| L2 | 4MB, 16-way | 12 cycles, 32 MSHRs, stream prefetch |
| L3 | 36MB, 16-way | 35 cycles, 64 MSHRs, tagged prefetch |
| TLBs | 160-entry L1, 4096-entry 8-way L2 TLB | |
| Memory | Row-buffer DRAM | 16-entry write-combining buffer |

The parameters follow public descriptions of the M4 Everest core; Apple
publishes none, so this preset is a large modern core rather than a
calibrated M4.

### Linux system

```python
from rvsim import presets

config = presets.linux(harts=8)                       # fast() cores
config = presets.linux(harts=1, core=presets.p550())  # any core
```

`linux()` keeps the core's pipeline, predictors and caches and replaces
the system around it: the memory map the bundled image expects, `harts`
harts kept coherent by a snoop-filter home agent over `interconnect`
(`mesh` by default), and four channels of DDR5-5600 (`memory="dram"`
keeps the core's controller). With `real_time=True`, the default, the CLINT
ticks at the device tree's 10 MHz timebase so the guest's clock keeps time.
See [Linux Boot](linux-boot.md).

### SiFive Performance P550

Based on published microarchitecture analysis (Chips and Cheese, SiFive specs).

```python
from rvsim import presets

config = presets.p550()
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Width | 3 | Triple-issue, 1.4 GHz (EIC7700X) |
| Backend | OutOfOrder | 72-entry ROB, 32-entry IQ, one load and one store AGU |
| Branch Predictor | Tournament | 9.1 KiB budget, 32-entry BTB, 16-entry RAS |
| L1D | 32KB, 4-way | 3-cycle load-to-use, 8 MSHRs |
| Prefetch | Load/store unit | Load stride prefetcher, 1 line ahead, keeps to the page; SiFive has not published the P550's prefetchers |
| L2 | 256KB, 8-way | 13-cycle load-to-use, 16 MSHRs |
| L3 | 4MB, 16-way | 38-cycle load-to-use, 32 MSHRs |
| Memory | LPDDR5 DRAM | 194 ns (272 cycles) random-access load-to-use |

The latencies are the ones Chips and Cheese measured on the HiFive Premier P550; `tools/diag/latency_probe.py --preset p550` reproduces them.

### ARM Cortex-A72

Based on publicly documented microarchitecture.

```python
from rvsim import presets

config = presets.cortex_a72()
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Width | 3 | Triple-issue, 1.5 GHz (Raspberry Pi 4) |
| Backend | OutOfOrder | 128-entry ROB, 66-entry IQ, 32-entry LQ, 16-entry SQ, one load and one store AGU |
| Branch Predictor | TAGE | 4 banks, 2048-entry tables, 4096-entry BTB, 31-entry RAS |
| L1I | 48KB, 3-way | NextLine prefetch |
| L1D | 32KB, 2-way | 4-cycle load-to-use, 8 MSHRs |
| Prefetch | Load/store unit | Loads into the L1D and 22 lines ahead into the L2, crossing pages through the TLB; store misses into the L2 (TRM §6.4.9) |
| L2 | 1MB, 16-way | 21-cycle load-to-use, 16 MSHRs |
| Memory | LPDDR4 DRAM | 162 ns (243 cycles) random-access load-to-use |

The latencies are the ones Chips and Cheese measured on the Cortex-A72 (Graviton), at the Pi 4's clock; `tools/diag/latency_probe.py --preset cortex_a72` reproduces them.

### Apple M1

Modeled after Apple's Firestorm (performance) core.

```python
from rvsim import presets

config = presets.m1()
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
from rvsim import Environment, presets

result = Environment(
    binary="software/bin/programs/qsort.elf",
    config=presets.p550(),
).run()

print(result.stats.query("ipc|stall|miss"))
```

### With Sweep

```python
from rvsim import Sweep, presets

results = Sweep(
    binaries=["software/bin/programs/qsort.elf"],
    configs={
        "P550": presets.p550(),
        "A72": presets.cortex_a72(),
        "M1": presets.m1(),
    },
).run(parallel=True)

results.compare(
    metrics=["ipc", "cycles", "core0.cache.l1d.misses", "core0.bp.committed.accuracy"]
)
```

### Deriving Variants

Use `replace()` to create variants of a benchmark config:

```python
from rvsim import Cache, presets

base = presets.p550()
wide = base.replace(width=4)
big_cache = base.replace(l1d=Cache("64KB", ways=8, latency=3, mshr_count=8))
```

## Creating Your Own Config

Follow the pattern in the existing configs. A config file should:

1. Define a function that returns a `Config` object
2. Assign it to a module-level `config` variable, which `rvsim --config FILE`, `rvsim bench --config FILE` and `rvsim.config.load_config()` look for (a `get_config()` function or a function named after the file also works)

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
