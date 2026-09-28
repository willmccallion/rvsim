# API Reference

## Config

The central configuration object. All parameters are flat — no nested objects.

```python
from rvsim import Config
```

### Constructor

See [Configuration](configuration.md) for the complete parameter reference.

```python
config = Config(
    width=4,
    branch_predictor=BranchPredictor.TAGE(),
    backend=Backend.OutOfOrder(rob_size=128),
    l1d=Cache("32KB", ways=8, latency=1, mshr_count=8),
    l2=Cache("256KB", ways=8, latency=10),
    hart_count=2,
    coherence=Coherence(interconnect=Interconnect.Ring()),
)
```

### Methods

#### `replace(**kwargs) -> Config`

Return a new Config with the given fields overridden. All other fields are preserved.

```python
base = Config(width=4, branch_predictor=BranchPredictor.TAGE())
narrow = base.replace(width=2)
inorder = base.replace(backend=Backend.InOrder())
```

#### `to_dict() -> dict`

Serialize to the nested dictionary format expected by the Rust backend. You normally don't need to call this directly.

---

## Environment

High-level interface for running a binary to completion and collecting statistics.

```python
from rvsim import Environment
```

### Constructor

```python
Environment(
    binary: str,                        # Path to RISC-V ELF binary
    config: Config | dict = Config(),   # Machine configuration
    disk: str | None = None,            # Optional disk image (VirtIO)
    load_addr: int = 0x8000_0000,       # Binary load address
)
```

### Methods

#### `run(quiet=True, limit=None, progress=0) -> Result`

Run the simulation to completion (or until `limit` cycles).

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `quiet` | `bool` | `True` | Suppress UART output |
| `limit` | `int` or `None` | `None` | Maximum cycles (None = unlimited) |
| `progress` | `int` | `0` | Print progress every N cycles (0 = no progress) |

```python
result = Environment("program.elf", config).run(limit=50_000_000)
```

---

## Result

Returned by `Environment.run()`.

### Properties

| Property | Type | Description |
|----------|------|-------------|
| `exit_code` | `int` | Program exit code (0 = success) |
| `stats` | `Stats` | All microarchitectural statistics |
| `wall_time_sec` | `float` | Host wall-clock time for the simulation |
| `binary` | `str` | Path to the binary that was run |
| `ok` | `bool` | `True` if `exit_code == 0` |

### Methods

#### `to_dict() -> dict`

JSON-serializable dictionary of all result fields.

---

## Simulator

Low-level fluent API for building and controlling a CPU instance tick-by-tick.

```python
from rvsim import Simulator
```

### Builder Methods

Each method returns `self` for chaining:

```python
cpu = (
    Simulator()
    .config(Config(width=4))       # Set configuration
    .binary("program.elf")         # Load ELF binary
    .kernel("Image")               # Optional: load kernel image
    .disk("rootfs.ext2")           # Optional: attach disk image
    .dtb("custom.dtb")            # Optional: use custom device tree
    .build()                       # Build and return Cpu instance
)
```

#### `config(path_or_config) -> Simulator`

Set the machine configuration. Accepts a `Config` object or a path to a Python config file.

#### `binary(path: str) -> Simulator`

Set the path to the RISC-V ELF binary to load.

#### `kernel(path: str) -> Simulator`

Set the kernel image path (for Linux boot).

#### `disk(path: str) -> Simulator`

Attach a VirtIO disk image.

#### `dtb(path: str) -> Simulator`

Use a custom device tree blob instead of the auto-generated one.

#### `build() -> Cpu`

Build the system, load the binary/kernel, and return a configured `Cpu` instance.

#### `run(limit=None, progress=0, stats_sections=None, output_stats=None) -> int`

Convenience method: build, run to completion, and return the exit code.

---

## Cpu

The live CPU instance returned by `Simulator.build()`. Provides tick-level control and state inspection.

### Control

#### `tick()`

Advance the simulation by one clock cycle.

#### `run(limit=None)`

Run until the program exits or `limit` cycles.

#### `run_until(pc=None, privilege=None)`

Run until the architectural PC (the next instruction to retire) equals the given address or the privilege level matches the given string (`"M"`, `"S"`, or `"U"`).

#### `save(path: str)`

Drain the pipelines and save a checkpoint to disk.

#### `restore(path: str)`

Restore from a checkpoint; the configuration may differ in anything but
hart count, RAM size and VLEN (see below).

### State Inspection

#### `pc -> int`

The architectural PC: the next instruction to retire. Fetch runs ahead of it. Writing it discards everything the pipeline had in flight and restarts fetch there.

Current program counter.

#### `regs[idx] -> int`

Read a general-purpose register by index. Use `reg` constants for named access:

```python
from rvsim import reg
print(cpu.regs[reg.A0])
print(cpu.regs[reg.SP])
print(cpu.regs[reg.RA])
```

#### `csrs[addr] -> int`

Read a CSR by address or by name, exactly as a CSR instruction on that
hart would read it (`sip` is the delegated view of `mip`, `time` is the
CLINT's counter, and so on). An address the hart does not implement
raises `KeyError`.

```python
from rvsim import csr
print(cpu.csrs[csr.MSTATUS])
print(cpu.csrs["satp"])
print(cpu.csrs[csr.SEPC])
```

`cpu.pc`, `cpu.regs` and `cpu.csrs` are hart 0's.

#### `harts[i] -> Hart`, `hart_count -> int`

Every hart's architectural state on a multi-core system
(`Config(hart_count=N)`): `pc` (read/write), `privilege`,
`instructions_retired`, `regs[idx]` and `csrs[addr]`.

```python
for hart in (cpu.harts[i] for i in range(cpu.hart_count)):
    print(hart.id, hex(hart.pc), hart.privilege, hart.regs[reg.A0])
```

#### `mem8[addr]`, `mem16[addr]`, `mem32[addr]`, `mem64[addr]`

Read memory at a physical address with the given width.

#### `trace -> bool`, `trace_filter(harts=None, cycles=None, trap_causes=None)`

`cpu.trace = True` arms the pipeline trace; events go to stderr through
the `RUST_LOG` target filter (`RUST_LOG=rvsim::trap=trace,rvsim::fetch=trace`
and so on), each inside a `hart{id=N}` span that is enabled whenever
`RUST_LOG` is set. The `rvsim::dma` target traces the virtio disk's
requests (start, each DMA phase, every transfer's return, completion)
without arming the pipeline trace. `trace_filter` narrows an armed trace to some harts, a
`(first, last)` cycle window, and, for every event that names a trap, a
list of `mcause` values (interrupt bit included); without a cause list
every trap prints except timer interrupts and ecalls.

```python
cpu.run(limit=19_000_000)
cpu.trace_filter(harts=[1], cycles=(19_100_000, 19_160_000), trap_causes=[1, 12])
cpu.trace = True
cpu.run(limit=60_000)
```

#### `save(path)`, `restore(path)`

A checkpoint holds RAM, the cycle counter, every hart's architectural
state (PC, privilege, integer, floating-point and vector registers, every
CSR, PMP entries, and its LR reservation) and the devices' registers
(CLINT timers and `mtime`, PLIC priorities, enables, thresholds and claims,
UART registers and unread input, the virtio disk's queue and every sector
the guest has written). `save` first
drains the machine the way gem5 does: speculative work is discarded,
committed stores still in the store buffers reach RAM, each hart is left at
its committed PC and a disk request in flight completes at once, so a run
that continues after a save is not cycle-identical to one without it.

A checkpoint restores into any configuration with the same hart count, RAM
size and VLEN, so a system can boot on a cheap configuration and continue
on a detailed one. It does not hold cache contents, TLBs, predictor state
or in-flight memory traffic: after a restore the caches, TLBs and the
coherence home agent start empty, as gem5's do, so warm the system up
before measuring. The disk's written sectors are replayed over the image
the restoring simulator loaded, so it must load the same image the
checkpoint was taken on; the image file itself is never modified. A
restore into a mismatched system, or onto a different disk image, raises
an error naming what differs and leaves the simulator untouched.

#### `pipeline_snapshot() -> PipelineSnapshot`

Capture the current pipeline state. Call `.visualize()` on the result to print an ASCII diagram, or `.render()` to get the string.

### Statistics

#### `stats -> Stats`

Access the current statistics (accumulated since the start of simulation,
the last checkpoint restore, or the last `reset_stats()`).

#### Measuring a region: `later - earlier`

Subtracting two `Stats` snapshots gives the stats of the region between
them, while the whole run's stats stay intact:

```python
start = cpu.stats
cpu.run(limit=1_000_000)
region = cpu.stats - start
print(region.ipc, region["core0.bp.committed.accuracy"])
```

Counters are subtracted and derived stats (IPC, miss rates, accuracies)
are recomputed from the differences. Histograms keep exact counts, sums and
means but report no minimum or maximum, which two cumulative snapshots
cannot recover. Subtracting a later snapshot, or across a `reset_stats()`,
raises `ValueError`.

#### `reset_stats()`

Zero every stat; `stats` then counts from here, as gem5's `m5 resetstats`
does. Prefer subtracting snapshots, which keeps the whole run's stats.

#### `stats_dumps -> list[tuple[int, Stats]]`, `stats_between(start, end) -> Stats`

Software running in the guest marks its own regions through the
sim-control device (see below): each dump it requests is kept here as a
labelled cumulative snapshot. `stats_between(start, end)` subtracts the
dump labelled `start` from the next one labelled `end`.

#### The sim-control device

A simulator-only MMIO device at `sim_control_base` (default `0x0010_2000`),
not in the device tree, with two 64-bit registers:

| Offset | Register | Access |
|---|---|---|
| `0x00` | `COMMAND` | write `1` to reset the stats, `2` to dump them labelled with `ARG`, `3` to end the simulation with `ARG` as the exit code, `4` to stop the host's `run_to` here with `ARG` as the label |
| `0x08` | `ARG` | read/write: the argument of the next command |

A guest writes `ARG`, then `COMMAND`. Under Linux, map the page through
`/dev/mem`. The command takes effect at the end of the cycle the write
reaches the device.

---

## Sweep

Parallel multi-configuration benchmarking framework.

```python
from rvsim import Sweep
```

### Constructor

```python
Sweep(
    binaries: list[str],                          # List of ELF paths
    configs: dict[str, Config | dict],            # Named configurations
)
```

### Methods

#### `run(parallel=True, limit=None, max_workers=None) -> SweepResults`

Execute all (binary, config) combinations.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `parallel` | `bool` | `True` | Run in parallel across CPU cores |
| `limit` | `int` or `None` | `None` | Per-run cycle limit |
| `max_workers` | `int` or `None` | `None` | Max parallel workers (None = CPU count) |

---

## SweepResults

Returned by `Sweep.run()`.

### Methods

#### `compare(metrics=None, baseline=None, col_header="")`

Print a comparison table.

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `metrics` | `list[str]` or `None` | `None` | Stat names to show (None = all) |
| `baseline` | `str` or `None` | `None` | Config name to use as baseline for ratios |
| `col_header` | `str` | `""` | Header label for the config column |

#### `__getitem__(binary: str) -> dict[str, Result]`

Access results for a specific binary.

---

## Stats

Dict subclass with filtering and comparison methods.

```python
from rvsim import Stats
```

### Methods

#### `query(pattern: str) -> Stats`

Filter statistics by regex or substring match (case-insensitive).

```python
result.stats.query("ipc|branch|miss")
result.stats.query("cache")
result.stats.query("stall")
```

#### `compare(other: Stats)`

Print a two-column comparison table.

#### `Stats.tabulate(rows: dict[str, Stats], title="") -> Table`

Build a comparison table from labeled Stats objects.

```python
print(Stats.tabulate({"A": stats_a, "B": stats_b}, title="Comparison"))
```

---

## ISA Utilities

### reg

Register index constants and lookup.

```python
from rvsim import reg

reg.A0        # 10
reg.SP        # 2
reg.RA        # 1
reg("a0")     # 10  (callable lookup)
reg.name(10)  # "a0" (reverse lookup)
```

### csr

CSR address constants and lookup.

```python
from rvsim import csr

csr.MSTATUS    # 0x300
csr.SATP       # 0x180
csr("mstatus") # 0x300 (callable lookup)
csr.name(0x300) # "mstatus" (reverse lookup)
```

### Disassemble

Fluent disassembler for RISC-V binaries.

```python
from rvsim import Disassemble

Disassemble().binary("program.elf").limit(20).print()
Disassemble().binary("program.elf").at(0x80001000, count=10).print()

# Single instruction
asm = Disassemble().inst(0x00a00513)  # "addi a0, zero, 10"
```
