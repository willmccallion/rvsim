# Scripts

Python utilities for benchmarking, analysis, and system setup.

## Directory Structure

- **benchmarks/**: Performance benchmarking and analysis
  - `p550/`: P550-style machine config (3-wide, 32KB L1, 256KB L2)
  - `m1/`: M1-style machine config (4-wide, 128KB L1, 4MB L2)
  - `tests/`: Comparison and smoke tests
- **setup/**: Installation and setup utilities
  - `boot_linux.py`: Downloads Buildroot, builds Linux kernel
- **analysis/**: Performance analysis tools (TODO: add genetic algorithm, etc.)

---

## Usage

**Run a machine benchmark:**
```bash
./target/release/sim script scripts/benchmarks/p550/run.py
./target/release/sim script scripts/benchmarks/m1/run.py [binary]
```

**Run a comparison:**
```bash
./target/release/sim script scripts/benchmarks/tests/compare_p550_m1.py
```

**Boot Linux:**
```bash
./target/release/sim script scripts/setup/boot_linux.py
```

---

## Python API

You can import configs from the benchmark packages if `scripts/` is on your PYTHONPATH (or if running via `sim script` which adds it).

```python
from benchmarks.p550.config import p550_config
from benchmarks.m1.config import m1_config

# Use with Environment
env = Environment(binary="...", config=p550_config())
```
