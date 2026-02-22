"""
rvsim simulator Python API.

A Python-first interface to the cycle-accurate RISC-V simulator:
1. **Configuration:** ``Config``, ``Cache``, ``BranchPredictor``, etc.
2. **Execution:** ``Cpu``, ``Simulator``.
3. **Experiments:** ``Environment``, ``Result``.
4. **Statistics:** ``Stats``, ``Table``.
5. **ISA:** ``reg``, ``csr``, ``Disassemble``.
6. **Pipeline:** ``PipelineSnapshot`` (from ``cpu.pipeline_snapshot()``).
"""

from importlib.metadata import version as _metadata_version

from .config import Config
from .experiment import Environment, Result
from .isa import Disassemble, csr, reg
from .objects import Cpu, Instruction, Simulator
from .pipeline import PipelineSnapshot
from .stats import Stats, Table
from .sweep import Sweep, SweepResults
from .types import (
    Backend,
    BranchPredictor,
    Cache,
    Fu,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
)


__version__ = _metadata_version("rvsim")

# Scrub submodule references and private imports that the import machinery
# pins as attributes. After this, `rvsim.objects` etc. raise AttributeError.
import sys as _sys

_rvsim_dict = _sys.modules[__name__].__dict__
for _name in (
    "config",
    "experiment",
    "isa",
    "objects",
    "pipeline",
    "stats",
    "sweep",
    "types",
    "_core",
    "_cli",
    "_metadata_version",
):
    _rvsim_dict.pop(_name, None)
del _sys, _rvsim_dict, _name


def version() -> str:
    """Return the installed rvsim version string."""
    return __version__


__all__ = [
    "__version__",
    "version",
    "Config",
    "BranchPredictor",
    "ReplacementPolicy",
    "Prefetcher",
    "MemoryController",
    "Backend",
    "Cache",
    "Fu",
    "Cpu",
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
