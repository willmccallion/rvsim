# Hardware Crates — Overview

Overview of the Rust hardware simulator library and its main modules.

**Source:** `hardware/src/` (re-exports in `hardware/src/lib.rs`).

---

## Top-Level Layout

```
hardware/src/
├── lib.rs          # Exports Config, Cpu, System
├── config.rs       # Simulator configuration (pipeline, caches, memory, BP, etc.)
├── common/         # Shared types and constants
├── core/           # CPU core (pipeline, arch state, units)
├── isa/            # Instruction set (decode, RV64I/M/A/F/D, RVC, privileged)
├── sim/            # Simulation loop and loader
├── soc/            # System-on-chip (bus, devices, memory controller)
└── stats.rs        # Statistics collection
```

---

## config.rs

Central configuration for the simulator: pipeline width, branch predictor type, BTB/RAS sizes, cache hierarchy (L1-I, L1-D, L2, L3), memory size and controller timing, TLB size, and system address map. This type is built from the Python config dict in the [bindings](bindings.md) via `conversion.rs`; see [Python configuration](../python/configuration.md) for the schema.

---

## common/

| File           | Purpose |
|----------------|---------|
| `addr.rs`     | Address types and masking. |
| `constants.rs`| Numeric and arch constants. |
| `data.rs`     | Data types (word, doubleword). |
| `error.rs`    | Error types (e.g., Trap). |
| `reg.rs`      | Register indices and helpers. |

---

## core/

| Submodule   | Path              | Purpose |
|-------------|-------------------|---------|
| **arch**    | `core/arch/`      | CSRs, GPR/FPR, privilege mode, traps. |
| **cpu**     | `core/cpu/`       | Execution, memory interface, trap handling. |
| **pipeline**| `core/pipeline/`  | 5-stage pipeline, latches, hazards, signals. |
| **units**   | `core/units/`     | ALU, BRU (branch predictors), cache, FPU, LSU, MMU (TLB, PTW), prefetchers. |

See [Pipeline](../../architecture/pipeline.md), [Branch prediction](../../architecture/branch_prediction.md), [Memory hierarchy](../../architecture/memory_hierarchy.md).

---

## isa/

Instruction set decoding and extension-specific opcodes. **decode.rs** is the main decoder; **instruction.rs** is the internal instruction type; **abi.rs** is for ABI/register names. Extensions: **rv64i**, **rv64m**, **rv64a**, **rv64f**, **rv64d**, **rvc**, **privileged**. See [ISA support](../../architecture/isa_support.md).

---

## sim/

- **loader.rs:** Load ELF/binary into memory and set entry PC; supports direct binary load and kernel boot (kernel + DTB + disk).
- **mod.rs:** Simulation driver (tick loop, device stepping). The CPU ticks; the loader is used by the bindings when starting a run or loading a kernel.

---

## soc/

Interconnect (bus), memory controller and buffer, and MMIO devices. **builder.rs** constructs the System with CPU, memory, and devices. **interconnect.rs** is the bus that routes requests by address. **devices/** contains CLINT, PLIC, UART, VirtIO disk, goldfish_rtc, syscon. See [SOC integration](soc_integration.md).

---

## stats.rs

Collects and exposes statistics: cycles, instructions retired, IPC, cache hits/misses (I-cache, D-cache, L2, L3), branch predictions/mispredictions, branch accuracy, stalls (mem, control, data), instruction counts by type, traps, etc. These are copied out and exposed to Python as **PyStats**; see [bindings](bindings.md) and Python [stats](https://github.com/.../rvsim/stats.py) (`.query("miss")`, `.query("branch")`).

---

## See also

- [Bindings](bindings.md) — how the hardware is exposed to Python.
- [SOC integration](soc_integration.md) — devices and bus.
- [Architecture](../../architecture/README.md) — pipeline, branch, memory, ISA.
