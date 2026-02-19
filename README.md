# RISC-V 64-bit System Simulator

**WARNING:** This project is currently broken due to ongoing refactors from version 0.9.0. Version 1.0.0 will be released once these refactors are complete and stable.

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
rvsim/
├── crates/              # Rust workspace
│   ├── hardware/        # CPU simulator core
│   └── bindings/        # Python bindings (PyO3)
├── rvsim/               # Python package for scripting
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

## Installation

**Python bindings** (via pip):
```bash
pip install rvsim
```

## Build from Source

**Requirements:**
- Rust toolchain (1.70+)
- `riscv64-unknown-elf-gcc` cross-compiler
- Python 3.10+ with maturin (for Python bindings)

### Quick Start

**Build everything:**
```bash
make build
```

**Run a benchmark:**
```bash
rvsim -f software/bin/benchmarks/qsort.bin
```

**Run a Python script:**
```bash
rvsim --script scripts/benchmarks/tests/smoke_test.py
```

### Available Make Targets

```bash
make help           # Show all available targets
make python         # Build and install Python bindings (editable)
make software       # Build libc and example programs
make test           # Run Rust tests
make lint           # Format check + clippy
make run-example    # Quick test (quicksort benchmark)
make clean          # Remove all build artifacts
```

### Python Scripting

The simulator supports Python scripting for hardware configuration and performance analysis:

```python
from rvsim import SimConfig, Simulator

# Configure a machine model
config = SimConfig.default()
config.pipeline.width = 4
config.pipeline.branch_predictor = "TAGE"
config.cache.l1_i.enabled = True
config.cache.l1_i.size_bytes = 65536

# Run a binary
Simulator().with_config(config).binary("software/bin/benchmarks/qsort.bin").run()
```

See **[docs/](docs/README.md)** for full API documentation and architecture details.

## Documentation

- **[Getting Started](docs/getting_started/README.md)** - Installation and quickstart guide
- **[Architecture](docs/architecture/README.md)** - CPU pipeline, memory system, ISA support
- **[API Reference](docs/api/README.md)** - Rust and Python API documentation
- **[Scripts](scripts/README.md)** - Performance analysis tools

## Linux Boot (Experimental)

⚠️ **Experimental Feature** — The simulator can boot Linux, though full boot is still in progress:

```bash
make linux          # Download and build Linux (takes time)
make run-linux      # Attempt to boot Linux
```

## License

Licensed under either of the following, at your option:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project shall be dual-licensed as above, without any
additional terms or conditions.
