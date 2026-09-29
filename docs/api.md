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

Builds a system from a configuration and workload, and runs it.

```python
from rvsim import Simulator
```

### Constructor

```python
Simulator(
    config: Config | dict | None = None,  # Machine configuration (default Config())
    *,
    binary: str | None = None,            # Bare-metal ELF to load
    elf_data: bytes | None = None,        # ...or its bytes
    kernel: str | None = None,            # Kernel image (Linux boot)
    firmware: str | None = None,          # OpenSBI fw_jump image; found under
                                          # software/linux/output if absent
    disk: str | None = None,              # VirtIO disk image
    dtb: str | None = None,               # Device tree; generated from config if absent
)
```

A `Simulator` is the live system: the methods below control it cycle by
cycle and inspect its state. The examples call it `cpu`.

### Control

#### `tick()`

Advance the simulation by one clock cycle.

#### `run(limit=None, progress=0, stats_sections=None) -> int | None`

Run until the program exits or `limit` cycles pass, and return the exit code,
or `None` if the limit was reached first. `progress=N` prints progress to
stderr every N cycles. `stats_sections=[]` prints every stats subject when the
run ends, and a list such as `["core0", "hart0"]` prints only those.

#### `run_until(predicate=None, *, pc=None, privilege=None, limit=None, chunk=10_000) -> int | None`

Run until the architectural PC (the next instruction to retire) equals `pc`,
the privilege level matches `privilege` (`"M"`, `"S"` or `"U"`), or
`predicate(cpu)` returns `True`; the predicate is checked every `chunk`
cycles. Returns the exit code if the program exited first, otherwise `None`.

#### `run_to(*, cycles=None, instructions=None, pc=None, guest_breaks=True, console_output=False) -> (str, int | None)`

Run until the first of these holds, checked every cycle, and say which:

| Returns | When |
|---|---|
| `("exit", code)` | the workload ended |
| `("break", label)` | guest software asked to stop (sim-control `BREAK`), if `guest_breaks` |
| `("console", None)` | a captured console holds output `read_console()` has not taken, if `console_output` |
| `("pc", hart)` | a hart's next instruction to retire is at `pc` (an address or a list of them) |
| `("instructions", None)` | `instructions` more have retired over all harts |
| `("cycles", None)` | `cycles` more cycles have passed |

It runs at least one cycle, so running on from a stop at a PC moves past
it. Ctrl-C interrupts it. `Session` builds its stop points on this.

#### `cycle -> int`, `instructions_retired -> int`

The cycle count and the instructions retired by every hart since the
system started. Unlike `stats.cycles`, they carry across checkpoints and
stats resets.

#### `read_console() -> str`, `write_console(text)`

With `console="captured"`, take what the guest has printed since the last
call, and type into its console.

#### `save(path)`, `restore(path)`

A checkpoint holds RAM (skipping 4 KiB pages of zeros), the cycle counter, every hart's architectural
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
`/dev/mem`, or use the image's `rvsim` tool (`rvsim dump-stats LABEL`,
`rvsim break LABEL`, `rvsim run START END CMD`); bare-metal programs
include `software/libc/rvsim.h` (`rvsim_dump_stats`, `rvsim_break`,
`rvsim_reset_stats`, `rvsim_exit`). The command takes effect at the end of
the cycle the write reaches the device.

---

## Session

Runs a workload in phases, the way gem5 users fast-forward, switch CPUs and
measure: get to the point of interest quickly, move onto the configuration
under study, warm it up, and measure regions. A session keeps the guest's
console, so a script can type into a Linux shell and wait for output.

```python
from rvsim import Session, presets

s = Session.linux(harts=8)
s.fast_forward(until=Session.LOGIN_SHELL)       # boots once; cached after that
s.switch(presets.linux(harts=8, core=my_core))  # the configuration under study
s.warm_up(command="coremark 0x0 0x0 0x66 20 7 1 2000")
r = s.measure("coremark 0x0 0x0 0x66 20 7 1 2000")
print(r.ipc, r.stats["core0.bp.committed.accuracy"], r.exit_code, r.console)
```

The same flow works for bare-metal programs:

```python
s = Session(my_core, binary="app.elf", fast_forward_config=cheap_core)
s.fast_forward(until=Instructions(1e9))
r = s.measure(until=Marker(2) | Exit())
```

### Constructors

#### `Session(config=None, *, binary=None, kernel=None, firmware=None, disk=None, dtb=None, fast_forward_config=None, cache_dir=None, echo=False, console_log=None, progress=False)`

A session on `config` running a bare-metal `binary`, or booting `kernel`
through OpenSBI `firmware` (`fw_jump.bin` beside the kernel by default).
Fast-forwards run on `fast_forward_config` (`config` unless given).
`echo` copies the console to stdout (or a stream), `console_log` writes it
to a file, and `progress` reports long runs on stderr.

#### `Session.linux(config=None, *, harts=None, image_dir=None, ...)`

A session on the image `make linux` builds. `config` defaults to
`presets.linux(harts)`. Its fast-forwards run on `presets.linux` with the
timer ticking every cycle, which shortens the boot's sleeps, keeping
`config`'s RAM, VLEN and ISA options, so the cached boot is shared by
every core configuration of the same system.

#### `Session.resume(path, config=None, **kwargs)`

Continue from a checkpoint `save` wrote, on `config` (the one it was saved
on by default), with its console and history. Refuses if the workload's
files changed since.

### Stop points

A run ends the moment one of its stops holds; `a | b` stops at whichever
comes first. Counts are relative to the run's start, and every run also
ends if the workload does.

| Stop | Holds |
|---|---|
| `Cycles(n)` | after `n` more cycles |
| `Instructions(n)` | once `n` more instructions have retired over all harts |
| `Pc(addr, ...)` | when any hart's next instruction to retire is at one of the addresses |
| `Marker(label=None)` | when guest software asks to stop (`rvsim break LABEL`, `rvsim_break(label)`); any label if `None` |
| `Console(pattern)` | when the console prints a regular-expression match, on the cycle it is printed; the match consumes the output |
| `Exit()` | when the workload ends |
| `When(predicate, every=100_000, name=None)` | when `predicate(session)` is true, checked every `every` cycles |
| `LOGIN_SHELL` / `LoginShell(user, password, login, prompt)` | at a Linux shell, after logging in |

A run returns a `Stopped`: the stop that held (`by`), `cycle`,
`instructions`, and `exit_code`, `hart`, `label` or `match` as it applies.

### Methods

#### `fast_forward(until, *, cache=True) -> Session`

Get to `until` quickly. The first time, the session runs there on the
fast-forward configuration and saves a checkpoint in the cache
(`$RVSIM_CACHE_DIR`, else `~/.cache/rvsim/checkpoints`). Later sessions
with the same history restore it in seconds. The cache key covers the
workload's files, everything the session did before, the fast-forward
configuration and the stop. Either way the session continues from the
checkpoint on its own configuration with caches, TLBs and predictors cold,
so results do not depend on whether the cache held the stop.
`last_fast_forward` says which happened. A predicate stop is cached only
if it has a `name`. Raises `WorkloadEnded` if the workload ends first.

#### `switch(config) -> Session`

Continue on `config`. It must show the guest the same system (harts, RAM,
memory map, VLEN, ISA options); for Linux, wrap a core configuration with
`presets.linux(core=...)`.

#### `run(until=None, *, every=None, on_every=None) -> Stopped`

Run to `until` (the workload's end by default), calling
`on_every(session)` every `every` cycles.

#### `warm_up(until=None, *, command=None) -> Session`

Run unmeasured to `until`, or through a shell `command`.

#### `measure(command=None, *, until=None, name=None) -> Region`

Measure a shell `command` (Linux) or the run to `until`. A command runs
under the guest's `rvsim run`, which snapshots the stats just before it
starts and just after it exits. A `Region` holds `stats` (only what
happened inside it), `console`, `exit_code`, `cycles`, `instructions`,
`ipc`, and `to_dict()`. The session's whole-run stats stay intact.

#### `send(text)`, `expect(pattern) -> re.Match`, `shell(command) -> ShellResult`

Type into the console; run until it prints `pattern`; run a shell command
and wait for its output and exit status.

#### `save(path) -> str`

Save a resume point: the checkpoint at `path` and the console and history
beside it in `path.json`. Saving drains the pipelines, as gem5 does.

#### `fork(configs) -> Iterator[tuple[str, Session]]`

Continue from this point once per configuration in `{name: config}`, each
as an independent session. This session is left where it was.

#### `sim`, `stats`, `console`, `cycle`, `instructions`, `config`

The running `Simulator`, its stats, everything the guest printed, the
counters since the system started, and the configuration the session runs
on. Changes made through `sim` directly are not part of the cache key's
history; fast-forward with `cache=False` after them.

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
