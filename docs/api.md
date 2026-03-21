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

Run until the PC matches the given address or the privilege level matches the given string (`"M"`, `"S"`, or `"U"`).

#### `save(path: str)`

Save a checkpoint to disk.

#### `restore(path: str)`

Restore from a checkpoint.

### State Inspection

#### `pc -> int`

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

Read a CSR by address. Use `csr` constants for named access:

```python
from rvsim import csr
print(cpu.csrs[csr.MSTATUS])
print(cpu.csrs[csr.SATP])
print(cpu.csrs[csr.SEPC])
```

#### `mem8[addr]`, `mem16[addr]`, `mem32[addr]`, `mem64[addr]`

Read memory at a physical address with the given width.

#### `pipeline_snapshot() -> PipelineSnapshot`

Capture the current pipeline state. Call `.visualize()` on the result to print an ASCII diagram, or `.render()` to get the string.

### Statistics

#### `stats -> Stats`

Access the current statistics (accumulated since the start of simulation or last checkpoint restore).

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
