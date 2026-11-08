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
)
from importlib.metadata import (
    version as _metadata_version,
)

from . import presets
from ._core import (
    CheckpointError,
    ConfigError,
    KernelPanic,
    LoadError,
    SimulationError,
    Stalled,
)
from .config import (
    Backend,
    BranchPredictor,
    Cache,
    Coherence,
    Config,
    Fu,
    HomeAgent,
    Interconnect,
    LoadPrefetcher,
    MemDepPredictor,
    MemoryController,
    PageBoundary,
    Prefetcher,
    ReplacementPolicy,
    StorePrefetcher,
)
from .experiment import Environment, Result
from .isa import Disassemble, Privilege, csr, reg
from .session import Region, Session, WorkloadEnded
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
from .simulator import (
    DebugEnd,
    Instruction,
    PipelineSnapshot,
    Program,
    Simulator,
    StopReason,
)
from .stats import Stats, Table
from .sweep import Sweep, SweepResults

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
    "LOGIN_SHELL",
    "AnyOf",
    "Backend",
    "BranchPredictor",
    "Cache",
    "CheckpointError",
    "Coherence",
    "Config",
    "ConfigError",
    "Console",
    "Cycles",
    "DebugEnd",
    "Disassemble",
    "Environment",
    "Exit",
    "Fu",
    "HomeAgent",
    "Instruction",
    "Instructions",
    "Interconnect",
    "KernelPanic",
    "LoadError",
    "LoadPrefetcher",
    "LoginShell",
    "Marker",
    "MemDepPredictor",
    "MemoryController",
    "PageBoundary",
    "Pc",
    "PipelineSnapshot",
    "Prefetcher",
    "Privilege",
    "Program",
    "Region",
    "ReplacementPolicy",
    "Result",
    "Session",
    "SimulationError",
    "Simulator",
    "Stalled",
    "Stats",
    "Stop",
    "StopReason",
    "Stopped",
    "StorePrefetcher",
    "Sweep",
    "SweepResults",
    "Table",
    "When",
    "WorkloadEnded",
    "__version__",
    "csr",
    "presets",
    "reg",
    "version",
]
