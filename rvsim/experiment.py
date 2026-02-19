"""
Reproducible experiment API for RISC-V architecture testing.

Provides:
- Environment: Immutable description of a run (binary, config, load address).
- Result: Structured result with exit code, stats, and wall time.
- run_experiment: Run one simulation and return a Result.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Dict, Optional, Union

from .config import Config, _config_to_dict
from .stats import Stats

from ._core import PySystem, PyCpu


@dataclass
class Environment:
    """Immutable description of a simulation run for reproducibility."""

    binary: str
    """Path to the RISC-V binary (bare-metal)."""

    config: Optional[Union[Config, Dict[str, Any]]] = None
    """Config or dict. If None, uses Config() defaults."""

    disk: Optional[str] = None
    """Optional disk image path."""

    load_addr: int = 0x8000_0000
    """Load address for the binary."""

    def get_config(self) -> Dict[str, Any]:
        """Returns the config as a dict for the Rust backend."""
        if self.config is not None:
            return _config_to_dict(self.config)
        return Config().to_dict()


@dataclass
class Result:
    """Structured result of a single run."""

    exit_code: int
    """Program exit code (e.g. 0 on success)."""

    stats: Stats = field(default_factory=lambda: Stats({}))
    """All stats as a Stats object."""

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


def run_experiment(env: Environment, quiet: bool = True) -> Result:
    """
    Run one simulation in a reproducible way.

    Example::

        env = Environment(binary="software/bin/benchmarks/qsort.bin")
        result = run_experiment(env)
        print(result.stats["ipc"], result.stats["cycles"])
    """
    config = env.get_config()
    t0 = time.perf_counter()
    try:
        sys_obj = PySystem(config, env.disk)
        with open(env.binary, "rb") as f:
            sys_obj.load_binary(f.read(), env.load_addr)
        cpu = PyCpu(sys_obj, config)
        exit_code = cpu.run()
        if exit_code is None:
            raise RuntimeError(
                "CPU run completed without exit code (should not happen without limit)"
            )
        stats_obj = cpu.get_stats()
        stats = stats_obj.to_dict()
    except Exception as e:
        err_msg = str(e)
        if not quiet:
            raise
        return Result(
            exit_code=-1,
            stats=Stats({"error": err_msg}),
            wall_time_sec=time.perf_counter() - t0,
            binary=env.binary,
        )
    wall = time.perf_counter() - t0
    return Result(
        exit_code=int(exit_code),
        stats=Stats(stats),
        wall_time_sec=wall,
        binary=env.binary,
    )
