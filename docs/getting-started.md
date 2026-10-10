# Getting Started

## Installation

### From PyPI (recommended)

```bash
pip install rvsim
```

Requires Python 3.10 or later. Pre-built wheels are available for Linux
x86-64 and run on any x86-64 CPU. The package includes the compiled Rust
simulator core, so no Rust toolchain is needed.

### From source

If you want to modify the simulator itself:

```bash
git clone https://github.com/willmccallion/rvsim
cd rvsim
make python
```

`make python` creates `.venv` if it does not exist, installs the
development requirements, and builds the Rust core with optimisations into
it as an editable package (`maturin develop --release`). Use the
interpreter in `.venv`, or activate it. The Rust toolchain version is
pinned in `rust-toolchain.toml`, and `rustup` installs it on first use.

The repository's `flake.nix` provides the whole toolchain (Rust, the
RISC-V cross-compiler, spike, `dtc`) for Nix users: `nix develop`.

### Building the example programs

The bundled programs are built with a bare-metal RISC-V cross-compiler.
The build expects the `riscv64-elf-` prefix (`riscv64-elf-gcc`,
`riscv64-elf-ld`); pass `TARGET=riscv64-unknown-elf` if your toolchain
uses that prefix instead.

```bash
make -C software                   # libc, programs and benchmarks into software/bin/
make -C software TARGET=riscv64-unknown-elf
```

The programs land in `software/bin/programs/`, the benchmarks in
`software/bin/benchmarks/`, and the multi-core tests in
`software/bin/multicore/`. `make linux` builds the Linux image (see
[Linux Boot](examples/linux-boot.md)).

## Your First Simulation

### Using the Python API

```python
from rvsim import Config, Environment

result = Environment(
    binary="software/bin/programs/qsort.elf",
    config=Config(width=4),
).run()

print(f"Exit code:    {result.exit_code}")
print(f"Cycles:       {result.stats['cycles']:,}")
print(f"IPC:          {result.stats['ipc']:.4f}")
print(f"Instructions: {result.stats['instructions_retired']:,}")
print(f"Host time:    {result.wall_time_sec:.2f} s")
```

`Environment.run()` returns a `Result` with the program's `exit_code`, its
`stats` and the host time it took; `result.ok` is true when the exit code
is 0.

### Using the command line

The `rvsim` command runs a program, a Python script or a kernel image,
chosen by the file:

```bash
rvsim software/bin/programs/qsort.elf            # run an ELF, print stats on exit
rvsim software/bin/programs/qsort.elf --watch    # live dashboard while it runs
rvsim examples/analysis/branch_predict.py        # run a script that uses the API
rvsim list                                       # list the bundled programs and benchmarks
rvsim bench coremark dhrystone                   # run benchmarks inside Linux
```

| Option | Meaning |
|--------|---------|
| `--limit N` | Stop after `N` cycles (`5M`, `500K` and `1G` are accepted) |
| `--watch` | Live dashboard: IPC, cache hit rates, branch accuracy, stalls |
| `--preset NAME` | Use a built-in configuration: `basic`, `fast`, `cortex_a72`, `m1`, `p550`, `rocket` or `boom` |
| `--config FILE` | Use the `config` (or `get_config()`) a Python file exports |
| `--harts N` | Run with `N` harts |
| `--json FILE` | Write the statistics as JSON |
| `--quiet` | Suppress all output, the program's included |
| `--no-stats` | Run without printing the statistics table |

`rvsim bench` boots Linux once on a configuration, caches the boot, and
measures each named benchmark (CoreMark, Dhrystone, Whetstone, STREAM,
mbw, `lat_mem_rd`, stress-ng) as its own region; `rvsim bench --help` lists
its options.

## Understanding the Output

### Stats

Every simulation produces a `Stats` object, a dictionary keyed by path,
with the microarchitectural counters of every component. Use `query()` to
filter it with a regular expression:

```python
result.stats.query(r"cache")              # every cache statistic
result.stats.query(r"^ipc$|\.bp\.")       # IPC and the branch predictor's
result.stats.query(r"miss")               # everything with "miss" in its path
```

Paths name where a statistic comes from: `core<N>.*` for one core's
pipeline, caches and predictor, `hart<N>.*` for one hardware thread's
architectural counts, `llc.*`, `memctrl0.*` and `coherence.*` for the
shared parts, and `system.*` for sums over the whole system. See
[Stats & Observability](architecture/stats.md).

### Key metrics

| Metric | What it means |
|--------|---------------|
| `cycles` | Total simulated clock cycles |
| `instructions_retired` | Instructions that committed |
| `ipc` | Instructions per cycle: `instructions_retired / cycles` |
| `core0.cache.l1d.misses` | L1 data cache misses (every cache also has `hits`, `miss_rate`, `mshr_hits`, `fills`, `evictions`, `writebacks`) |
| `core0.bp.committed.accuracy` | Fraction of committed control instructions predicted correctly (0 to 1) |
| `core0.pipeline.stalls.data` | Cycles issue found nothing whose operands were ready |
| `core0.pipeline.stalls.fu_structural` | Cycles a ready instruction waited for a free functional unit |
| `core0.pipeline.stalls.dispatch` | Cycles rename had no ROB or issue-queue room |
| `core0.pipeline.stalls.squash` | Cycles rename waited while commit squashed the ROB after a misprediction or trap |
| `core0.pipeline.flushes.total` | Squashes taken, also split by cause (`branch`, `system`, `mem_violations`) |

## Comparing Configurations

The simplest way to compare configurations is `Stats.tabulate()`:

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
    rows[name] = r.stats.query(r"^ipc$|bp\.committed\.(accuracy|mispredicts)")

print(Stats.tabulate(rows, title="Branch Predictor Comparison"))
```

## Parallel Sweeps

For larger experiments, `Sweep` runs every (binary, config) combination
across the host's CPU cores:

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
    metrics=["ipc", "core0.cache.l1d.misses"],
    baseline="L1=8KB",
    col_header="L1D Size",
)
```

This runs all 12 combinations in parallel and prints a table with each
configuration's ratio to the baseline.

## Running in Phases

`Session` runs one workload in phases: fast-forward to a point of
interest, switch to the configuration under study, warm it up, and
measure regions. Fast-forwards are cached, so the second run of an
experiment restores the boot in seconds:

```python
from rvsim import Session

s = Session.linux(harts=1)
s.fast_forward(until=Session.LOGIN_SHELL)     # boots once, then cached
r = s.measure("coremark 0x0 0x0 0x66 200 7 1 2000")
print(r.cycles, r.instructions, r.ipc)
```

See [Linux Boot](examples/linux-boot.md) and
[Linux Benchmarks](examples/linux-benchmarks.md).

## Low-Level Control

For fine-grained control, drive a `Simulator` directly:

```python
from rvsim import Simulator, Config, reg, csr

cpu = Simulator(Config(width=4), binary="software/bin/programs/qsort.elf")

# Tick 1000 cycles
for _ in range(1000):
    cpu.tick()

# Run until a specific PC, or until code runs in user mode
cpu.run_until(pc=0x80001234)
cpu.run_until(privilege="U")

# Inspect state
print(f"PC: {cpu.pc:#x}")
print(f"a0: {cpu.regs[reg.A0]:#x}")
print(f"sp: {cpu.regs[reg.SP]:#x}")
print(f"mstatus: {cpu.csrs[csr.MSTATUS]:#x}")
print(f"mem[0x80001000]: {cpu.mem64[0x80001000]:#x}")

# Every hart, in a multi-core run
for hart in cpu.harts:
    print(hart.pc, hart.privilege)

# Pipeline visualisation
cpu.pipeline_snapshot().visualize()

# Checkpoint and restore (the pipelines are drained first)
cpu.save("checkpoint.bin")
cpu.restore("checkpoint.bin")
```

## Next Steps

- [Configuration Reference](configuration.md): every parameter of the simulated machine
- [API Reference](api.md): the Python classes
- [Design](architecture/design.md): how the simulator is structured and why
- [Analysis Scripts](examples/analysis-scripts.md): the bundled design-space exploration scripts
