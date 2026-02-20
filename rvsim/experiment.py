"""
Reproducible experiment API for RISC-V architecture testing.

Provides:
- Environment: Immutable description of a run (binary, config, load address).
- Result: Structured result with exit code, stats, and wall time.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Union

from .config import Config, _config_to_dict
from .stats import Stats, _compare_flat, _compare_matrix

from ._core import PySystem, PyCpu
from .objects import Cpu


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

    def run(
        self, quiet: bool = True, limit: Optional[int] = None, progress: int = 0
    ) -> Result:
        """
        Run the simulation and return a :class:`Result`.

        Args:
            quiet: Suppress exceptions and return error Result instead.
            limit: Max cycles to simulate. ``None`` means unlimited.

        Example::

            env = Environment(binary="software/bin/benchmarks/qsort.bin")
            result = env.run()
            print(result.stats["ipc"], result.stats["cycles"])
        """
        config = self.get_config()
        t0 = time.perf_counter()
        try:
            sys_obj = PySystem(config, self.disk)
            with open(self.binary, "rb") as f:
                data = f.read()
            entry, tohost = sys_obj.load_elf(data)
            cpu = Cpu(PyCpu(sys_obj, config))
            cpu.pc = entry
            if tohost is not None:
                cpu._cpu.set_direct_mode(False)
                cpu._cpu.set_htif_range(tohost, 16)
            exit_code = cpu.run(limit=limit, progress=progress)
            if exit_code is None and limit is None:
                raise RuntimeError(
                    "CPU run completed without exit code (should not happen without limit)"
                )
            stats = cpu.stats
        except Exception as e:
            err_msg = str(e)
            if not quiet:
                raise
            return Result(
                exit_code=-1,
                stats=Stats({"error": err_msg}),
                wall_time_sec=time.perf_counter() - t0,
                binary=self.binary,
            )
        wall = time.perf_counter() - t0
        return Result(
            exit_code=int(exit_code) if exit_code is not None else -1,
            stats=Stats(stats),
            wall_time_sec=wall,
            binary=self.binary,
        )


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

    @staticmethod
    def compare(
        results: Dict[str, Any],
        *,
        metrics: Optional[List[str]] = None,
        baseline: Optional[str] = None,
    ) -> None:
        """
        Print a comparison table for experiment results.

        Args:
            results: Either ``dict[str, Result]`` (single binary, multiple configs)
                     or ``dict[str, dict[str, Result]]`` (multi-binary x multi-config).
            metrics: Specific metric names to show. If None, shows a default set.
            baseline: Config name to normalize against (shows speedup ratios).
        """
        first_val = next(iter(results.values()))
        is_nested = isinstance(first_val, dict)

        if is_nested:
            _compare_matrix(results, metrics=metrics, baseline=baseline)
        else:
            _compare_flat(results, metrics=metrics, baseline=baseline)
