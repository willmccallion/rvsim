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

### The sim-control device

A simulator-only MMIO device at `sim_control_base` (default `0x0010_2000`),
not in the device tree, with two 64-bit registers:

| Offset | Register | Access |
|---|---|---|
| `0x00` | `COMMAND` | write `1` to reset the stats, `2` to dump them labelled with `ARG`, `3` to end the simulation with `ARG` as the exit code, `4` to stop the host's `run_to` here with `ARG` as the label |
| `0x08` | `ARG` | read/write: the argument of the next command |

A guest writes `ARG`, then `COMMAND`. Under Linux, map the page through
`/dev/mem`, or use the image's `rvsim` tool (`rvsim dump-stats LABEL`,
`rvsim break LABEL`, `rvsim run START END CMD`); bare-metal programs
include `software/libc/rvsim.h` (`rvsim_dump_stats`, `rvsim_break`,
`rvsim_reset_stats`, `rvsim_exit`). The command takes effect at the end of
the cycle the write reaches the device.

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
