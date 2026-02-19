# Rust–Python Bindings (PyO3)

How the Rust simulator is exposed to Python via PyO3.

**Source:** `bindings/src/`

---

## Overview

The **bindings** crate builds a Python extension module **`rvsim`** that the CLI injects into `sys.modules` when running `sim script <path>`. It wraps the Rust `Cpu` and `System`, converts Python config dicts to Rust `Config`, and exposes stats and device access. Scripts use the high-level Python API in `rvsim/` (e.g., `Simulator`, `Environment`, `run_experiment`), which call into these bindings.

---

## Module Layout

```
bindings/src/
├── lib.rs         # PyModule registration; add_class for PyCpu, PySystem, PyStats, PyMemory, devices
├── conversion.rs  # py_dict_to_config: Python dict → Rust Config
├── cpu.rs         # PyCpu
├── system.rs      # PySystem
├── memory.rs      # PyMemory
├── stats.rs       # PyStats
├── utils.rs       # Helpers (e.g., version)
└── devices/       # PyUart, PyPlic, PyVirtioBlock
```

---

## PyCpu (`cpu.rs`)

- **`new(system, config_dict)`:** Takes ownership of the `PySystem` and builds a Rust `Cpu` from the converted config. The system can only be attached to one CPU.
- **`load_kernel(kernel_path, config_dict, dtb_path=None)`:** Calls `loader::setup_kernel_load` and sets `direct_mode = false` for OS boot.
- **`tick()`:** Runs one cycle.
- **`get_stats()`** → **PyStats:** Returns a copy of the CPU statistics.
- **`get_pc()`** → `u64`: Current PC.
- **`run(py)`:** Runs until exit (checks Python signals periodically, flushes stdout for UART). Returns exit code when the program exits (e.g., ECALL with specific a7).

---

## PySystem (`system.rs`)

- Constructs the Rust `System` (SoC: bus, memory, devices) from a config dict and optional disk image path.
- Used to create a **PyCpu**; after that the system is moved into the CPU.

---

## Conversion (`conversion.rs`)

**`py_dict_to_config(py, config_dict)`** converts the Python config (nested dict with keys `general`, `system`, `memory`, `cache`, `pipeline`) into the Rust `Config` type. Field names and structure match the Python [SimConfig](../python/configuration.md) `to_dict()` output so that cache, pipeline, branch predictor (TAGE, Perceptron, Tournament), and memory settings are applied correctly.

---

## PyStats (`stats.rs`)

Wraps the Rust stats (e.g., cycles, instructions_retired, ipc, cache hits/misses, branch stats, stalls, instruction counts). Exposed to Python as a dict-like object; the Python layer wraps it in **StatsObject** with **`.query(pattern)`** for filtering (e.g., `query("miss")`, `query("branch")`). See `rvsim/stats.py`.

---

## PyMemory (`memory.rs`)

Exposes memory/loader interface to Python (e.g., for loading binaries or inspecting memory if needed). Used by the loader path and any script that needs to load data at an address.

---

## Devices (`devices/`)

**PyUart**, **PyPlic**, **PyVirtioBlock** expose device state or control to Python when needed (e.g., for debugging or scripting). The SoC attaches these devices to the bus; see [soc_integration](soc_integration.md).

---

## Usage from Python

Scripts run with `sim script scripts/...` get `rvsim` on `sys.path` and use `rvsim` objects that ultimately call **PySystem**, **PyCpu**, **PyStats**. They do not typically construct **PyCpu** or **PySystem** directly; they use **Simulator**, **Environment**, and **run_experiment** in [simulation_objects](../python/simulation_objects.md) and [scripting](../python/scripting.md).

---

## See also

- [Hardware crates](hardware_crates.md) — what is wrapped.
- [SOC integration](soc_integration.md) — devices and system.
- [Python configuration](../python/configuration.md) — config schema.
- [Python simulation objects](../python/simulation_objects.md) — Simulator, Environment, run_experiment.
