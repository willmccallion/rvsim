"""
RISC-V simulator Python API.

This package provides a Python-first interface to the cycle-accurate RISC-V simulator. It provides:
1. **Configuration:** `SimConfig` and `config_to_dict` for building machine models (cache, BP, pipeline).
2. **Execution:** `System`, `P550Cpu`, `simulate`, and `Simulator` for running binaries and scripts.
3. **Experiments:** `Environment`, `ExperimentResult`, and `run_experiment` for reproducible sweeps.
4. **Statistics:** `StatsObject` for performance metrics and sectioned output.
5. **Rust bindings:** `PySystem`, `CPU` (PyCpu), and `Memory` from the riscv_emulator extension.
"""
import riscv_emulator
from .config import SimConfig, config_to_dict
from .objects import System, P550Cpu, simulate, get_default_config, Simulator
from .experiment import Environment, ExperimentResult, run_experiment
from .stats import StatsObject

PySystem = riscv_emulator.PySystem
CPU = riscv_emulator.PyCpu
Memory = riscv_emulator.PyMemory

import sys
sys.modules[f"{__name__}._core"] = riscv_emulator

__all__ = [
    "SimConfig",
    "config_to_dict",
    "System",
    "P550Cpu",
    "simulate",
    "get_default_config",
    "Environment",
    "ExperimentResult",
    "run_experiment",
    "StatsObject",
    "PySystem",
    "CPU",
    "Memory",
]
