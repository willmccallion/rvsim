"""
Reproducible experiment API for RISC-V architecture testing.

Provides:
- Environment: Immutable description of a run (binary, config, load address).
- Result: Structured result with exit code, stats, and wall time.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any

__all__ = ["Environment", "Result"]

from .config import Config
from .config._config import _config_to_dict
from .simulator import Simulator
from .stats import Stats, _compare_flat, _compare_matrix


@dataclass
class Environment:
    """Immutable description of a simulation run for reproducibility."""

    binary: str
    """Path to the RISC-V binary (bare-metal)."""

    config: Config | dict[str, Any] | None = None
    """Config or dict. If None, uses Config() defaults."""

    disk: str | None = None
    """Optional disk image path."""

    load_addr: int = 0x8000_0000
    """Load address for the binary."""

    args: list[str] = field(default_factory=list)
    """The program's command line after its name, for a Linux user program
    run in SE mode (its ``argv[0]`` is ``binary``)."""

    env: list[str] | None = None
    """The program's environment as ``NAME=value`` entries, for SE mode;
    empty by default."""

    mode: str | None = None
    """``"se"`` to run a Linux user program with its syscalls served by the
    simulator, ``"bare"`` for a bare-metal image; ``None`` decides from the
    ELF (an entry below RAM with no ``tohost`` symbol is a user program)."""

    script: str | None = None
    """The file the program's readfile pseudo-operation reads."""

    outdir: str | None = None
    """The directory the program's writefile pseudo-operation writes into,
    created if missing."""

    def get_config(self) -> dict[str, Any]:
        """Returns the config as a dict for the Rust backend."""
        if self.config is not None:
            return _config_to_dict(self.config)
        return Config().to_dict()

    def run(
        self, quiet: bool = True, limit: int | None = None, progress: int = 0
    ) -> Result:
        """
        Run the simulation and return a `Result`.

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
            with open(self.binary, "rb") as f:
                elf_data = f.read()
            cpu = Simulator(
                config,
                elf_data=elf_data,
                args=[self.binary, *self.args],
                env=list(self.env or []),
                mode=self.mode,
                disk=self.disk,
                script=self.script,
                outdir=self.outdir,
            )
            exit_code = cpu.run(limit=limit, progress=progress)
            if exit_code is None and limit is None:
                raise RuntimeError(
                    "CPU run completed without exit code (should not happen without limit)"
                )
            stats = Stats.from_core(cpu.stats)
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
            stats=stats,
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

    @property
    def ok(self) -> bool:
        """``True`` if the program exited with code 0."""
        return self.exit_code == 0

    def to_dict(self) -> dict[str, Any]:
        """JSON-serializable dict for saving and comparison."""
        return {
            "exit_code": self.exit_code,
            "binary": self.binary,
            "wall_time_sec": self.wall_time_sec,
            "stats": dict(self.stats),
        }

    @staticmethod
    def compare(
        results: dict[str, Any],
        *,
        metrics: list[str] | None = None,
        baseline: str | None = None,
        col_header: str = "",
    ) -> None:
        """
        Print a comparison table for experiment results.

        Args:
            results: Either ``dict[str, Result]`` (single binary, multiple configs)
                     or ``dict[str, dict[str, Result]]`` (multi-binary x multi-config).
            metrics: Specific metric names to show. If None, shows a default set.
            baseline: Config name to normalize against (shows speedup ratios).
            col_header: Label for the config-name column (e.g. "size", "width").
        """
        first_val = next(iter(results.values()))
        is_nested = isinstance(first_val, dict)

        if is_nested:
            _compare_matrix(
                results, metrics=metrics, baseline=baseline, col_header=col_header
            )
        else:
            _compare_flat(
                results, metrics=metrics, baseline=baseline, col_header=col_header
            )
