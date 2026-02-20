"""
rvsim simulator Python API.

A Python-first interface to the cycle-accurate RISC-V simulator:
1. **Configuration:** ``Config``, ``Cache``, ``BranchPredictor``, etc.
2. **Execution:** ``System``, ``Cpu``, ``Simulator``, ``simulate``.
3. **Experiments:** ``Environment``, ``Result``, ``run_experiment``.
4. **Statistics:** ``Stats``, ``compare``.
5. **ISA:** ``reg``, ``csr``, ``disassemble``.
"""

from importlib.metadata import version as _metadata_version

from ._core import disassemble
from .config import Config
from .experiment import Environment, Result, run_experiment
from .isa import Disassemble, csr, csr_name, reg, reg_name
from .objects import Cpu, Instruction, Simulator, System, simulate
from .stats import Stats, compare
from .types import (
    Backend,
    BranchPredictor,
    Cache,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
)

__version__ = _metadata_version("rvsim")


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
    "System",
    "Cpu",
    "Simulator",
    "simulate",
    "Instruction",
    "Environment",
    "Result",
    "run_experiment",
    "Stats",
    "compare",
    "disassemble",
    "reg",
    "reg_name",
    "csr",
    "csr_name",
    "Disassemble",
]
