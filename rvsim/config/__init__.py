"""Simulator configuration.

``Config`` holds every parameter at the top level; the component types
(``Cache``, ``BranchPredictor``, ``Backend`` …) describe the parts that have
more than one field.
"""

from ._config import Config, load_config
from .backend import Backend, Fu
from .branch import BranchPredictor, MemDepPredictor
from .coherence import Coherence, HomeAgent, Interconnect
from .memory import Cache, MemoryController, Prefetcher, ReplacementPolicy

__all__ = [
    "Backend",
    "BranchPredictor",
    "Cache",
    "Coherence",
    "Config",
    "Fu",
    "HomeAgent",
    "Interconnect",
    "MemDepPredictor",
    "MemoryController",
    "Prefetcher",
    "ReplacementPolicy",
    "load_config",
]
