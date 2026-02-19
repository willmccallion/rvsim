# Simulation Objects (Python Interface)

Python-side simulation and CPU abstractions sitting on top of the Rust core.

**Source:** `rvsim/` — especially `objects.py`, `cpu/`, `experiment.py`.

---

## Overview

The Python layer provides a high-level API to build and drive the [Rust core](../rust/bindings.md). It uses **SimObject** as a base for systems and CPUs; **Simulator** for kernel boot and large runs; and **Environment** / **run_experiment()** for script-based runs with IPC and stats query.

---

## Simulation entry points (`objects.py`)

### `Simulator`

Fluent API to set up and run a full system (typically for kernel boot). Used by `scripts/setup/boot_linux.py`.

- **`config(path_or_obj)`:** Load a machine config from a file (e.g., `"scripts/m1/config.py"`) or a `SimConfig` object.
- **`kernel(path)`**, **`disk(path)`**, **`dtb(path)`**: Set paths for kernel image, disk image (rootfs), and device tree blob.
- **`kernel_mode()`**: Enable kernel boot mode (non-direct mode).
- **`run()`**: Start simulation and return exit code (calls **`PyCpu::run()`** in the backend).

### `Environment` and `run_experiment()`

Descriptive run entry point for benchmarks and single-binary experiments.

- **`Environment(binary=..., config=...)`**: Describes a run. `config` can be a `SimConfig` or a dict.
- **`run_experiment(env, quiet=False)`**: Runs the environment and returns a **Result** object.
- **Result:** Contains `exit_code`, `stats` (a **StatsObject**), and any errors.

---

## CPU wrappers (`cpu/`)

These wrappers build the Rust `Cpu` from a `System` and a `SimConfig`.

### `P550Cpu`, `M1Cpu` (`cpu/o3.py`)

Wrappers for O3 (pipelined) CPU models.
- **`__init__(system, config=None, trace=False, ...)`**: Takes a system and an optional config.
- **`create()`**: Builds the backend **`PyCpu`** using the system’s Rust system and the configuration.

### `AtomicCpu` (`cpu/atomic.py`)

Non-pipelined functional CPU model for fast execution or debugging.
- Uses **`SimConfig.minimal()`** as a base.

---

## Statistics (`stats.py`)

**StatsObject** wraps the dictionary of stats from the backend.
- **`.query(pattern)`**: Filters stats by key (e.g., `query("miss")`, `query("branch")`). Pattern is a case-insensitive regex or substring.
- **Keys (typical):** `cycles`, `instructions_retired`, `ipc`, `icache_hits/misses`, `dcache_hits/misses`, `l2_hits/misses`, `branch_predictions/mispredictions`, `branch_accuracy_pct`, `stalls_mem/control/data`, and per-type instruction counts (e.g., `inst_alu`, `inst_load`).

---

## See also

- [Rust bindings](../rust/bindings.md) — what these objects call.
- [Configuration](configuration.md) — `SimConfig` and params.
- [Scripting](scripting.md) — how scripts use these objects.
