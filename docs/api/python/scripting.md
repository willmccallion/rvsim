# Scripting Guide

How to run and extend the simulator via scripts in `scripts/`: machine configs, tests, and setup.

**Source:** `scripts/` — see also [scripts/README.md](https://github.com/.../scripts/README.md).

---

## Script Layout

Scripts are organized by machine (e.g., `m1/`, `p550/`) or workflow (`setup/`, `tests/`).

- **`scripts/p550/`**, **`scripts/m1/`**: Machine definitions. `config.py` defines the hardware (L1/L2 sizes, pipeline width, BP); `run.py` is the entry point to run a benchmark on that machine.
- **`scripts/setup/`**: `boot_linux.py` downloads Buildroot, builds Linux, and boots it (uses the M1 config).
- **`scripts/tests/`**: `smoke_test.py` (basic run check) and `compare_p550_m1.py` (performance comparison).

Run scripts via the CLI from the **repo root**:

```bash
./target/release/sim script scripts/<path/to/script>.py [args...]
```

---

## Machine Scripts (p550, m1)

Each machine dir has:
- **`config.py`**: A module that defines the machine (e.g., `p550_config()`). It must expose a **`config`** attribute for `Simulator.config()` to find.
- **`run.py`**: A script that uses the config and **`run_experiment()`** to run a benchmark.

```bash
# Run qsort on P550
./target/release/sim script scripts/p550/run.py

# Run a specific binary on M1
./target/release/sim script scripts/m1/run.py software/bin/benchmarks/raytracer.bin
```

---

## Performance Comparison

**`scripts/tests/compare_p550_m1.py`** runs the same binary on both P550 and M1 configs and prints differences in cycles, IPC, cache/branch misses, and branch accuracy. It uses `from p550.config import p550_config` to get the machine models.

```bash
./target/release/sim script scripts/tests/compare_p550_m1.py software/bin/benchmarks/qsort.bin
```

---

## Linux Boot (`boot_linux.py`)

**`scripts/setup/boot_linux.py`** automates the full Linux stack:
1. Downloads Buildroot.
2. Builds the kernel (Image), OpenSBI (fw_jump.bin), and rootfs (disk.img).
3. Compiles the device tree (system.dtb).
4. Boots in the simulator using the **M1** config and the [Simulator](simulation_objects.md) API.

```bash
./target/release/sim script scripts/setup/boot_linux.py
```

---

## Writing Your Own Script

1. **Imports:** Start with `from rvsim import Environment, run_experiment, SimConfig`.
2. **Path:** Add `scripts/` to `sys.path` if you want to import `p550` or `m1` configs.
3. **Run:**
```python
import os, sys
# Add scripts dir so we can import machine configs
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from p550.config import p550_config

config = p550_config()
env = Environment(binary="software/bin/benchmarks/qsort.bin", config=config)
result = run_experiment(env)
print(f"IPC: {result.stats['ipc']}")
print(result.stats.query("miss"))
```

---

## See also

- [Simulation objects](simulation_objects.md) — `Simulator`, `Environment`, `run_experiment`.
- [Configuration](configuration.md) — `SimConfig` schema and params.
- [Quickstart](../../getting_started/quickstart.md) — CLI and first run.
