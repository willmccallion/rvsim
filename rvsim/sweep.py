"""
Parallel multi-config x multi-binary sweep runner.

Provides ``Sweep`` for running multiple configurations against multiple binaries
across CPU cores and collecting structured results.
"""

from __future__ import annotations

import os
from concurrent.futures import ProcessPoolExecutor
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Union

__all__ = ["Sweep", "SweepResults"]

from .config import Config
from .experiment import Environment, Result


def _run_one(args: tuple) -> tuple:
    """Worker function for parallel execution. Must be top-level for pickling."""
    binary, config_name, config, limit = args
    env = Environment(binary=binary, config=config)
    result = env.run(quiet=True, limit=limit)
    return (binary, config_name, result)


@dataclass
class SweepResults:
    """Structured results from a sweep run.

    Organised as ``results[binary_name][config_name] = Result``.
    """

    data: Dict[str, Dict[str, Result]] = field(default_factory=dict)

    def compare(
        self,
        *,
        metrics: Optional[List[str]] = None,
        baseline: Optional[str] = None,
        col_header: str = "",
    ) -> None:
        """Print a comparison table across all configs and binaries."""
        Result.compare(
            self.data, metrics=metrics, baseline=baseline, col_header=col_header
        )

    def __getitem__(self, key: str) -> Dict[str, Result]:
        return self.data[key]

    def __repr__(self) -> str:
        n_bins = len(self.data)
        n_cfgs = len(next(iter(self.data.values()), {}))
        return f"SweepResults({n_bins} binaries x {n_cfgs} configs)"


class Sweep:
    """Run multiple configs x binaries in parallel across CPU cores.

    Example::

        from rvsim import Sweep, Config

        results = Sweep(
            binaries=["qsort.elf", "dhrystone.elf"],
            configs={
                "baseline": Config(width=2),
                "wide": Config(width=4),
            },
        ).run(parallel=True, limit=100_000_000)

        results.compare()
    """

    def __init__(
        self,
        binaries: List[str],
        configs: Dict[str, Union[Config, Dict[str, Any]]],
    ):
        self.binaries = binaries
        self.configs = configs

    def run(
        self,
        *,
        parallel: bool = True,
        limit: Optional[int] = None,
        max_workers: Optional[int] = None,
    ) -> SweepResults:
        """Execute all (binary, config) combinations.

        Args:
            parallel: Use multiple processes. ``False`` runs sequentially.
            limit: Maximum cycles per run. ``None`` = unlimited.
            max_workers: Max parallel workers. ``None`` = number of CPUs.

        Returns:
            :class:`SweepResults` with per-binary, per-config results.
        """
        # Build work items
        work: List[tuple] = []
        for binary in self.binaries:
            for config_name, config in self.configs.items():
                work.append((binary, config_name, config, limit))

        # Execute
        if parallel and len(work) > 1:
            with ProcessPoolExecutor(max_workers=max_workers) as pool:
                raw_results = list(pool.map(_run_one, work))
        else:
            raw_results = [_run_one(w) for w in work]

        # Organise into nested dict
        data: Dict[str, Dict[str, Result]] = {}
        for binary, config_name, result in raw_results:
            bin_key = os.path.basename(binary)
            if bin_key not in data:
                data[bin_key] = {}
            data[bin_key][config_name] = result

        return SweepResults(data=data)
