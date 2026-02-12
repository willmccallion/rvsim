"""
Reproducible experiment API for RISC-V architecture testing.

This module provides:
1. **Environment:** Immutable description of a run (binary path, config, load address, optional disk).
2. **ExperimentResult:** Structured result with exit code, stats dict, and wall time for logging and comparison.
3. **run_experiment:** Single entry point to run one simulation and return an ExperimentResult; normalizes config and catches errors.
"""
from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Dict, Optional, Union

from .config import SimConfig, config_to_dict
from .objects import get_default_config
from .stats import StatsObject

try:
    from . import PySystem, CPU
except ImportError:
    from riscv_emulator import PySystem
    CPU = None
    for _ in (0,):
        try:
            CPU = __import__("riscv_emulator").PyCpu
        except Exception:
            pass
    if CPU is None:
        raise ImportError("riscv_emulator.PyCpu not available")


@dataclass
class Environment:
    """Immutable description of a simulation run for reproducibility."""

    binary: str
    """Path to the RISC-V binary (bare-metal)."""

    config: Optional[Union[SimConfig, Dict[str, Any]]] = None
    """SimConfig or dict. If None, uses get_default_config() (base config)."""

    disk: Optional[str] = None
    """Optional disk image path (for OS boot; usually None for bare-metal)."""

    load_addr: int = 0x8000_0000
    """Load address for the binary."""

    def get_config(self) -> Dict[str, Any]:
        """Returns the config as a dict for the Rust backend; uses get_default_config() if config is None."""
        if self.config is not None:
            return config_to_dict(self.config)
        return get_default_config()


@dataclass
class ExperimentResult:
    """Structured result of a single run: exit code, stats dict, and wall time."""

    exit_code: int
    """Program exit code (e.g. 0 on success)."""

    stats: StatsObject = field(default_factory=lambda: StatsObject({}))
    """All stats as a StatsObject (cycles, instructions_retired, ipc, cache stats, etc.)."""

    wall_time_sec: float = 0.0
    """Wall-clock time of the run in seconds."""

    binary: str = ""
    """Binary path (from Environment)."""

    def to_dict(self) -> Dict[str, Any]:
        """JSON-serializable dict for saving and comparison."""
        return {
            "exit_code": self.exit_code,
            "binary": self.binary,
            "wall_time_sec": self.wall_time_sec,
            "stats": dict(self.stats),
        }


def run_experiment(env: Environment, quiet: bool = True) -> ExperimentResult:
    """
    Run one simulation in a reproducible way. Returns structured result for logging/comparison.

    Example:
        env = Environment(binary="software/bin/benchmarks/qsort.bin")
        result = run_experiment(env)
        print(result.stats["ipc"], result.stats["cycles"])
        # Query stats
        print(result.stats.query("cache"))
        with open("run.json", "w") as f:
            json.dump(result.to_dict(), f, indent=2)
    """
    config = env.get_config()
    t0 = time.perf_counter()
    try:
        sys_obj = PySystem(config, env.disk)
        with open(env.binary, "rb") as f:
            sys_obj.load_binary(f.read(), env.load_addr)
        cpu = CPU(sys_obj, config)
        exit_code = cpu.run()
        stats_obj = cpu.get_stats()
        stats = stats_obj.to_dict()
    except Exception as e:
        err_msg = str(e)
        if not quiet:
            raise
        return ExperimentResult(
            exit_code=-1,
            stats=StatsObject({"error": err_msg}),
            wall_time_sec=time.perf_counter() - t0,
            binary=env.binary,
        )
    wall = time.perf_counter() - t0
    stats_plain = StatsObject(stats)
    return ExperimentResult(
        exit_code=int(exit_code),
        stats=stats_plain,
        wall_time_sec=wall,
        binary=env.binary,
    )
