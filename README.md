# RISC-V 64-bit System Simulator

A cycle-accurate system simulator for the RISC-V 64-bit architecture (RV64IMAFD). Features a 5-stage pipelined CPU, comprehensive memory hierarchy, and can boot Linux (experimental).

## Technologies Used

* **Languages:** Rust (Simulator), C (Libc/Software), RISC-V Assembly, Python (Analysis)
* **Concepts:** Pipelining, Virtual Memory (SV39), Cache Coherence, Branch Prediction, OS Development
* **Tools:** Make, GCC Cross-Compiler, Cargo

## Key Implementation Details

### CPU Core (Rust)

* **5-Stage Pipeline:** Implements Fetch, Decode, Execute, Memory, and Writeback stages with full data forwarding and hazard detection.
* **Branch Prediction:** Multiple swappable predictors including Static, GShare, Tournament, Perceptron, and TAGE (Tagged Geometric History).
* **Floating Point:** Support for single and double-precision floating-point arithmetic (F/D extensions).

### Memory System

* **Memory Management Unit (MMU):** Implements SV39 virtual addressing with translation lookaside buffers (iTLB and dTLB).
* **Cache Hierarchy:** Configurable L1, L2, and L3 caches supporting LRU, PLRU, and Random replacement policies.
* **DRAM Controller:** Simulates timing constraints including row-buffer conflicts, CAS/RAS latency, and precharge penalties.

### Example Programs (C & Assembly)

* **Custom Libc:** A minimal standard library written from scratch (includes `printf`, `malloc`, string manipulation).
* **Benchmarks:** Complete programs including chess engine, raytracer, quicksort, and performance microbenchmarks.
* **User Programs:** Various test applications (Game of Life, Mandelbrot, 2048, etc.).

### Performance Analysis

* **Automated Benchmarking:** Python scripts to sweep hardware parameters (e.g., cache size vs. IPC) and visualize bottlenecks.
* **Design Space Exploration:** Hardware configuration comparison and performance analysis tools.

## Project Structure

```
riscv-system/
├── crates/              # Rust workspace
│   ├── hardware/        # CPU simulator core
│   ├── bindings/        # Python bindings (PyO3)
│   └── cli/             # CLI tool (sim)
├── riscv_sim/           # Python package for scripting
├── software/            # System software
│   ├── libc/            # Custom C standard library
│   └── linux/           # Linux boot configuration
├── examples/            # Example programs
│   ├── benchmarks/      # Performance benchmarks
│   └── programs/        # User applications
├── scripts/             # Analysis and utilities
│   ├── benchmarks/      # Performance analysis scripts
│   └── setup/           # Installation helpers
└── docs/                # Documentation
```

## Build and Run

**Requirements:**
- Rust toolchain (1.70+)
- `riscv64-unknown-elf-gcc` cross-compiler
- Python 3.8+ (for scripting)

### Quick Start

**Build everything:**
```bash
make all
```

**Run a benchmark:**
```bash
./target/release/sim run -f software/bin/benchmarks/qsort.bin
```

**Run Python analysis scripts:**
```bash
./target/release/sim script scripts/benchmarks/tests/smoke_test.py
```

### Available Make Targets

```bash
make help           # Show all available targets
make simulator      # Build Rust simulator only
make software       # Build libc and examples
make test           # Run Rust tests
make clippy         # Run linter
make run-example    # Quick test (quicksort)
make clean          # Remove all build artifacts
```

### Python Scripting

The simulator supports Python scripting for hardware configuration and performance analysis:

```python
from riscv_sim import *

# Configure system
cpu = O3CPU()
cpu.branch_predictor = TournamentBP()
system = System(cpu)

# Run simulation
system.run("software/bin/benchmarks/qsort.bin")

# Query statistics
stats = system.query()
print(f"IPC: {stats['ipc']}")
print(f"Branch accuracy: {stats['branch_predictor.accuracy']}")
```

See **[docs/](docs/README.md)** for full API documentation and architecture details.

## Documentation

- **[Getting Started](docs/getting_started/README.md)** - Installation and quickstart guide
- **[Architecture](docs/architecture/README.md)** - CPU pipeline, memory system, ISA support
- **[API Reference](docs/api/README.md)** - Rust and Python API documentation
- **[Scripts](scripts/README.md)** - Performance analysis tools

## Linux Boot (Experimental)

The simulator can boot Linux, though full boot is still in progress:

```bash
make linux          # Download and build Linux (takes time)
make run-linux      # Attempt to boot Linux
```

## License

This project is licensed under the MIT License — see [LICENSE](LICENSE).
