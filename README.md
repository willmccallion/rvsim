# rvsim

A cycle-accurate RISC-V 64-bit system simulator (RV64IMAFDC) written in Rust with Python bindings. Features a 10-stage superscalar pipeline, multi-level cache hierarchy, branch prediction, and virtual memory. Can boot Linux (experimental).

## Pipeline

10-stage in-order pipeline with configurable superscalar width:

```
Fetch1 → Fetch2 → Decode → Rename → Issue → Execute → Memory1 → Memory2 → Writeback → Commit
```

- **Superscalar:** configurable width (1, 2, 4+)
- **Reorder buffer** for in-order commit with tag-based register scoreboard
- **Store buffer** with store-to-load forwarding
- **Branch prediction:** Static, GShare, Tournament, Perceptron, TAGE

## Memory System

- **MMU:** SV39 virtual addressing with separate iTLB and dTLB
- **Cache hierarchy:** configurable L1i, L1d, L2, L3 with LRU/PLRU/FIFO/Random replacement
- **Prefetchers:** next-line, stride, stream, tagged
- **DRAM controller:** row-buffer aware timing (CAS/RAS/precharge)

## ISA Support

RV64IMAFDC — base integer, multiply/divide, atomics, single/double float, compressed instructions. Privileged ISA with M/S/U modes, traps, CSRs, and CLINT timer.

Passes all 134 tests in the [`riscv-software-src/riscv-tests`](https://github.com/riscv-software-src/riscv-tests) ISA suite (rv64ui, rv64um, rv64ua, rv64uf, rv64ud, rv64uc, rv64mi, rv64si).

## Quick Start

**Install:**
```bash
pip install rvsim
```

**Run a program:**
```python
from rvsim import Config, Environment

result = Environment(binary="software/bin/programs/mandelbrot.elf").run()
print(result.stats.query("ipc|branch"))
```

**From the command line:**
```bash
make build
rvsim -f software/bin/programs/qsort.elf
```

## Python API

```python
from rvsim import Config, Cache, BranchPredictor, Environment, Stats

config = Config(
    width=4,
    branch_predictor=BranchPredictor.TAGE(),
    l1d=Cache("64KB", ways=8),
    l2=Cache("512KB", ways=16, latency=10),
)

result = Environment(binary="software/bin/programs/qsort.elf", config=config).run()

# Query specific stats
print(result.stats.query("branch"))
print(result.stats.query("miss"))

# Compare configurations
rows = {}
for w in [1, 2, 4]:
    cfg = Config(width=w, uart_quiet=True)
    r = Environment(binary="software/bin/programs/qsort.elf", config=cfg).run()
    rows[f"w{w}"] = r.stats.query("ipc|cycles")
print(Stats.tabulate(rows, title="Width Scaling"))
```

## Analysis Scripts

Modular scripts for design-space exploration in `scripts/analysis/`:

| Script | Purpose |
|--------|---------|
| `width_scaling.py` | IPC vs pipeline width |
| `branch_predict.py` | Compare branch predictor accuracy |
| `cache_sweep.py` | L1 D-cache size vs miss rate |
| `inst_mix.py` | Instruction class breakdown |
| `stall_breakdown.py` | Memory/control/data stall cycles |

```bash
rvsim scripts/analysis/width_scaling.py --bp TAGE --widths 1 2 4
rvsim scripts/analysis/branch_predict.py --width 2 --programs maze qsort
rvsim scripts/analysis/cache_sweep.py --sizes 4KB 16KB 64KB
```

Machine model benchmarks in `scripts/benchmarks/`:

```bash
rvsim scripts/benchmarks/p550/run.py
rvsim scripts/benchmarks/m1/run.py
rvsim scripts/benchmarks/tests/compare_p550_m1.py
```

## Project Structure

```
rvsim/
├── crates/
│   ├── hardware/        # Simulator core (Rust)
│   │   └── src/
│   │       ├── core/    # CPU, pipeline, execution units
│   │       ├── isa/     # RV64IMAFDC decode and execution
│   │       ├── sim/     # Simulator driver, binary loader
│   │       └── soc/     # Bus, UART, PLIC, VirtIO, CLINT
│   └── bindings/        # Python bindings (PyO3)
├── rvsim/               # Python package
├── examples/
│   ├── programs/        # C and assembly source
│   └── benchmarks/      # Microbenchmarks and synthetic workloads
├── software/
│   ├── libc/            # Custom minimal C standard library
│   └── linux/           # Linux boot configuration
├── scripts/
│   ├── analysis/        # Design-space exploration scripts
│   ├── benchmarks/      # Machine model configs (P550, M1)
│   └── setup/           # Linux build helpers
└── docs/                # Architecture and API documentation
```

## Build from Source

**Requirements:**
- Rust (2024 edition)
- Python 3.10+ with maturin
- `riscv64-unknown-elf-gcc` cross-compiler (for building example programs)

```bash
make build          # Build Python bindings (editable)
make software       # Build libc and example programs
make test           # Run Rust tests
make lint           # Format check + clippy
make clean          # Remove all build artifacts
```

## Linux Boot (Experimental)

The simulator can boot Linux through OpenSBI. Full boot is still in progress.

```bash
make linux          # Download and build Linux via Buildroot
make run-linux      # Boot Linux
```

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
