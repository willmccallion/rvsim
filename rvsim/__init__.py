"""
rvsim simulator Python API.

A Python-first interface to the cycle-accurate RISC-V simulator:
1. **Configuration:** ``Config``, ``Cache``, ``BranchPredictor``, ``MemDepPredictor``, etc.
2. **Execution:** ``Simulator``.
3. **Experiments:** ``Environment``, ``Result``.
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
from .objects import Instruction, Simulator
from .pipeline import PipelineSnapshot
from .stats import Stats, Table
from .sweep import Sweep, SweepResults
from .types import (
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

# Scrub submodule references and private imports that the import machinery
# pins as attributes. After this, `rvsim.objects` etc. raise AttributeError.
import sys as _sys

_rvsim_dict = _sys.modules[__name__].__dict__
for _name in (
    "config",
    "experiment",
    "presets",
    "isa",
    "objects",
    "pipeline",
    "stats",
    "sweep",
    "types",
    "_core",
    "_cli",
    "_metadata_version",
    "_PackageNotFoundError",
):
    _rvsim_dict.pop(_name, None)
del _sys, _rvsim_dict, _name


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
    "Stats",
    "Table",
    "reg",
    "csr",
    "Disassemble",
    "Sweep",
    "SweepResults",
]
