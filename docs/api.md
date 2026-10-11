# API Reference

Generated from the package's docstrings, including the compiled
extension's, so it describes the installed version. The examples call a
running simulator `cpu`.

## Configuration

::: rvsim.Config
    options:
      members: [replace, to_dict]

See [Configuration](configuration.md) for every parameter.

::: rvsim.Cache

::: rvsim.BranchPredictor

::: rvsim.MemDepPredictor

::: rvsim.Backend

::: rvsim.Fu

::: rvsim.MemoryController

::: rvsim.Prefetcher

::: rvsim.ReplacementPolicy

::: rvsim.Coherence

::: rvsim.HomeAgent

::: rvsim.Interconnect

::: rvsim.presets

## Running a binary

::: rvsim.Environment

::: rvsim.Result

## The simulator

::: rvsim.Simulator

### Pseudo-operations

A program marks regions, dumps or resets the stats, stops the host's run
or ends it with [pseudo-operations](pseudo-ops.md): gem5's m5op
instruction, which `software/include/rvsim.h` wraps as `rvsim_dump_stats`,
`rvsim_reset_stats`, `rvsim_break` and `rvsim_exit`. The dumps come back
through `Simulator.stats_dumps()`, the stops through `run_to` and the
Session API's markers.

::: rvsim.Instruction

::: rvsim.PipelineSnapshot

## Sessions

::: rvsim.Session

::: rvsim.Region

::: rvsim.Stopped

::: rvsim.WorkloadEnded

### Stop points

A run ends the moment one of its stops holds; `a | b` stops at whichever
comes first. Counts are relative to the run's start, and every run also
ends if the workload does.

::: rvsim.Stop

::: rvsim.Cycles

::: rvsim.Instructions

::: rvsim.Pc

::: rvsim.Marker

::: rvsim.Console

::: rvsim.Exit

::: rvsim.When

::: rvsim.AnyOf

::: rvsim.LoginShell

`LOGIN_SHELL` is `LoginShell()` with the image's defaults.

### `rvsim bench`

The benchmark suite inside Linux from the command line:

```bash
rvsim bench                              # every benchmark, fast core, 8 harts
rvsim bench coremark stream --warm       # warm each benchmark before measuring it
rvsim bench --config my_core.py --harts 4 --json results.json
rvsim bench --list
```

It boots once per system (cached), places the core in the Linux system
with `presets.linux(core=...)`, and prints each benchmark's cycles,
instructions, IPC and branch, L1D, L2 and LLC misses per thousand
instructions.

## Sweeps

::: rvsim.Sweep

::: rvsim.SweepResults

## Statistics

::: rvsim.Stats

::: rvsim.Table

## ISA utilities

### `reg` and `csr`

Register and CSR lookups: constants, name lookup, and the reverse.

```python
from rvsim import reg, csr

reg.A0          # 10
reg("a0")       # 10
reg.name(10)    # "a0"
csr.MSTATUS     # 0x300
csr("mstatus")  # 0x300
csr.name(0x300) # "mstatus"
```

::: rvsim.Disassemble
