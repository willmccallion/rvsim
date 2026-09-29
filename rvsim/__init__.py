"""
rvsim simulator Python API.

A Python-first interface to the cycle-level RISC-V simulator:
1. **Configuration:** ``Config``, ``Cache``, ``BranchPredictor``, ``MemDepPredictor``, etc.
2. **Execution:** ``Simulator``.
3. **Experiments:** ``Environment``, ``Result``; ``Session`` for phased
   runs (fast-forward, switch configuration, measure) with the stop points
   ``Cycles``, ``Instructions``, ``Pc``, ``Marker``, ``Console``, ``Exit``,
   ``When`` and ``LOGIN_SHELL``.
4. **Statistics:** ``Stats``, ``Table``.
5. **ISA:** ``reg``, ``csr``, ``Disassemble``.
6. **Pipeline:** ``PipelineSnapshot`` (from ``cpu.pipeline_snapshot()``).
"""

from importlib.metadata import (
    PackageNotFoundError as _PackageNotFoundError,
    version as _metadata_version,
)

from . import presets
from .config import Config
from .experiment import Environment, Result
from .isa import Disassemble, csr, reg
from .simulator import Instruction, PipelineSnapshot, Simulator
from .session import Region, Session, WorkloadEnded
from .stats import Stats, Table
from .session.stops import (
    LOGIN_SHELL,
    AnyOf,
    Console,
    Cycles,
    Exit,
    Instructions,
    LoginShell,
    Marker,
    Pc,
    Stop,
    Stopped,
    When,
)
from .sweep import Sweep, SweepResults
from .config import (
    Backend,
    BranchPredictor,
    Cache,
    Coherence,
    Fu,
    HomeAgent,
    Interconnect,
    MemDepPredictor,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
)


try:
    __version__ = _metadata_version("rvsim")
except _PackageNotFoundError:
    # Package not installed via pip / maturin develop. Common when running
    # from source or in a parallel-test subprocess that races the install.
    # Use a sentinel rather than failing import — version is observability,
    # not load-bearing.
    __version__ = "0.0.0+dev"


def version() -> str:
    """Return the installed rvsim version string."""
    return __version__


__all__ = [
    "__version__",
    "version",
    "presets",
    "Config",
    "BranchPredictor",
    "MemDepPredictor",
    "ReplacementPolicy",
    "Prefetcher",
    "MemoryController",
    "Backend",
    "Cache",
    "Coherence",
    "HomeAgent",
    "Interconnect",
    "Fu",
    "Simulator",
    "Instruction",
    "PipelineSnapshot",
    "Environment",
    "Result",
    "Session",
    "Region",
    "WorkloadEnded",
    "Stop",
    "Stopped",
    "Cycles",
    "Instructions",
    "Pc",
    "Marker",
    "Console",
    "Exit",
    "When",
    "AnyOf",
    "LoginShell",
    "LOGIN_SHELL",
    "Stats",
    "Table",
    "reg",
    "csr",
    "Disassemble",
    "Sweep",
    "SweepResults",
]
