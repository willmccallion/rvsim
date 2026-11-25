"""Simulator configuration.

``Config`` holds every parameter at the top level; the component types
(``Cache``, ``BranchPredictor``, ``Backend`` …) describe the parts that have
more than one field.
"""

from ._config import Config, load_config
from .backend import Backend, Fu
from .branch import BranchPredictor, MemDepPredictor, MicroBtb
from .coherence import Coherence, HomeAgent, Interconnect
from .console import ConsoleMode
from .memory import (
    Cache,
    LoadPrefetcher,
    MemoryController,
    PageBoundary,
    Prefetcher,
    ReplacementPolicy,
    StorePrefetcher,
    WriteStreaming,
)

__all__ = [
    "Backend",
    "BranchPredictor",
    "Cache",
    "Coherence",
    "Config",
    "ConsoleMode",
    "Fu",
    "HomeAgent",
    "Interconnect",
    "LoadPrefetcher",
    "MemDepPredictor",
    "MemoryController",
    "MicroBtb",
    "PageBoundary",
    "Prefetcher",
    "ReplacementPolicy",
    "StorePrefetcher",
    "WriteStreaming",
    "load_config",
]
