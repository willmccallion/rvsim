# Benchmark Configurations

`Config()` is the default machine, and `rvsim.presets` holds the others. The analysis examples use them; `replace()` derives your own, so a variant names only what it changes.

## Available Configurations

| Configuration | What it is |
|--------|------------|
| `Config()` | A desktop out-of-order core of the mid-2020s, with three cache levels, every prefetcher and DDR5-5600 |
| `presets.inorder()` | The same machine around a dual-issue in-order core |
| `presets.rocket()`, `presets.boom()` | Chipyard's Rocket and Medium BOOM as their RTL builds them ([Error against Rocket](../rtl-rocket.md), [Error against BOOM](../rtl-boom.md)) |
| `presets.linux(harts, core=...)` | A core, `Config()` by default, placed in the system that boots the bundled Linux image |

`--preset` names them on the command line: `default`, `inorder`, `rocket`
and `boom`.

### The default machine

```python
from rvsim import BranchPredictor, Config

config = Config()
variant = Config(l3=None, branch_predictor=BranchPredictor.GShare())
```

| Parameter | Value | Notes |
|-----------|-------|-------|
| Clock | 4.0 GHz | `cpu_clock_mhz` and `clock_hz` |
| Width | 6 | Every stage; a mispredicted branch costs 14 cycles |
| Backend | OutOfOrder | 320-entry ROB, 128-entry IQ, 128-entry LQ, 72-entry SQ, 3 load and 2 store ports, 224 integer, 192 FP and 96 vector physical registers |
| Functional units | 4 ALU (1 cycle), 1 MUL (3), 1 DIV (14), 2 each of FP add (3), multiply (4) and FMA (4), 1 FP divide/sqrt (13), 2 branch, 3 address units | Vector: VLEN 256, 4 lanes, 2 integer, 2 FP, 2 memory and 1 permute pipe |
| Branch predictor | 64KB TAGE-SC-L with ITTAGE | 8K-entry 8-way BTB, 32-entry RAS, store-set memory dependence predictor |
| L1I | 32 KiB, 8-way, PLRU | Next-line prefetch, 8 MSHRs |
| L1D | 48 KiB, 12-way, PLRU | 5-cycle load-to-use, 16 MSHRs, next-line prefetch, 16-entry write-combining buffer |
| L2 | 1 MiB, 8-way, private | 14-cycle load-to-use, 32 MSHRs, stream prefetch |
| L3 | 32 MiB, 16-way, shared | 49-cycle load-to-use, 64 MSHRs, non-inclusive |
| Load/store prefetch | Load/store unit | Loads 4 lines ahead into the L1D and 16 into the L2, crossing pages through the TLB; store misses streamed into the L2 |
| TLBs | 64-entry L1 TLBs, 2048-entry 8-way L2 TLB | |
| Memory | DDR5-5600, two channels | 90 ns random-access load-to-use |

The values are typical of a current desktop core rather than taken from
any one chip; the load-to-use latencies are what
`tools/diag/latency_probe.py --preset default` measures.

### In-order

```python
from rvsim import presets

config = presets.inorder()
```

`Config(width=2, backend=Backend.InOrder())`: the default machine's clock,
front end, predictors, caches, prefetchers, TLBs and memory around a
dual-issue in-order core with two integer ALUs and one of every other
unit, so comparing it with `Config()` compares only the cores.

### Linux system

```python
from rvsim import presets

config = presets.linux(harts=8)                          # Config() cores
config = presets.linux(harts=1, core=presets.inorder())  # any core
```

`linux()` keeps the core's pipeline, predictors and caches and replaces
the system around it: the memory map the bundled image expects, `harts`
harts kept coherent by a snoop-filter home agent over `interconnect`
(`mesh` by default), and four channels of DDR5-5600 (`memory="dram"`
keeps the core's controller). With `real_time=True`, the default, the CLINT
ticks at the device tree's 10 MHz timebase so the guest's clock keeps time.
See [Linux Boot](linux-boot.md).

## Calibrated configurations

`tools/diag/calibration.py` holds two configurations fitted to measured
hardware, for the hardware comparisons on the
[Linux Benchmarks](linux-benchmarks.md) page: their structure sizes are
the published ones and their latencies the measured ones. They are not
models of the cores they are named after, whose RTL is not published, so
they are not presets; from the repository root:

```python
from tools.diag.calibration import cortex_a72, p550

config = p550()
```

### SiFive Performance P550

Based on published microarchitecture analysis (Chips and Cheese, SiFive specs).

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

## Using Benchmark Configs

### With Environment

```python
from rvsim import Config, Environment

result = Environment(
    binary="software/bin/programs/qsort.elf",
    config=Config(),
).run()

print(result.stats.query("ipc|stall|miss"))
```

### With Sweep

```python
from rvsim import Config, Sweep, presets

results = Sweep(
    binaries=["software/bin/programs/qsort.elf"],
    configs={
        "default": Config(),
        "inorder": presets.inorder(),
        "boom": presets.boom(),
    },
).run(parallel=True)

results.compare(
    metrics=["ipc", "cycles", "core0.cache.l1d.misses", "core0.bp.committed.accuracy"]
)
```

### Deriving Variants

Use `replace()` to create variants of a benchmark config:

```python
from rvsim import Cache, Config

base = Config()
wide = base.replace(width=8)
big_cache = base.replace(l1d=Cache("64KB", ways=8, latency=3, mshr_count=8))
no_l3 = base.replace(l3=None)
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
