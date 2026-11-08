"""The flat ``Config``: every parameter at the top level, assembled into
the nested dict the Rust core expects."""

from __future__ import annotations

import copy
import dataclasses
from dataclasses import dataclass
from typing import Any

from .._core import check_config
from ._core_defaults import MACHINE
from ._serialize import _config_to_dict_impl
from ._units import _parse_size
from .backend import _AnyBackend, _backend_from
from .branch import (
    _AnyBranchPredictor,
    _AnyMemDepPredictor,
    _branch_predictor_from,
    _mem_dep_predictor_from,
)
from .coherence import Coherence
from .console import ConsoleMode
from .memory import (
    Cache,
    LoadPrefetcher,
    StorePrefetcher,
    _AnyInclusionPolicy,
    _AnyMemoryController,
    _cache_from,
    _controller_from,
    _load_prefetcher_from,
    _store_prefetcher_from,
)

_GENERAL = MACHINE["general"]
_SYSTEM = MACHINE["system"]
_MEMORY = MACHINE["memory"]
_CACHES = MACHINE["cache"]
_PIPELINE = MACHINE["pipeline"]
_ISA = MACHINE["isa"]

# The default machine's components, as templates: every Config stores its
# own copy.
_DEFAULT_BRANCH_PREDICTOR = _branch_predictor_from(_PIPELINE)
_DEFAULT_BACKEND = _backend_from(_PIPELINE)
_DEFAULT_MEM_DEP_PREDICTOR = _mem_dep_predictor_from(_PIPELINE)
_DEFAULT_L1I = _cache_from(_CACHES["l1_i"])
_DEFAULT_L1D = _cache_from(_CACHES["l1_d"])
_DEFAULT_L2 = _cache_from(_CACHES["l2"])
_DEFAULT_L3 = _cache_from(_CACHES["l3"])
_DEFAULT_INCLUSION_POLICY = getattr(Cache, _CACHES["inclusion_policy"])()
_DEFAULT_LOAD_PREFETCHER = _load_prefetcher_from(_CACHES["load_prefetcher"])
_DEFAULT_STORE_PREFETCHER = _store_prefetcher_from(_CACHES["store_prefetcher"])
_DEFAULT_MEMORY_CONTROLLER = _controller_from(_MEMORY)
_DEFAULT_COHERENCE = Coherence()

# The fields holding component objects, which each Config copies.
_COMPONENT_FIELDS = (
    "branch_predictor",
    "backend",
    "mem_dep_predictor",
    "l1i",
    "l1d",
    "l2",
    "l3",
    "inclusion_policy",
    "load_prefetcher",
    "store_prefetcher",
    "memory_controller",
    "coherence",
)


@dataclass(kw_only=True, eq=False, repr=False)
class Config:
    """
    Full simulator configuration with flat parameter access.

    The core checks the configuration as it is built: one it cannot read
    or would refuse to build raises ``ValueError`` naming the field.

    Example::

        from rvsim import Config, Cache, BranchPredictor, Prefetcher

        cfg = Config(
            width=4,
            branch_predictor=BranchPredictor.Tage(),
            l1i=Cache("128KB", ways=8, prefetcher=Prefetcher.NextLine(degree=2)),
            l1d=Cache("128KB", ways=8, prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
            l2=Cache("4MB", ways=16, latency=12),
        )
    """

    # Pipeline
    width: int = _PIPELINE["width"]
    fetch_width: int | None = _PIPELINE["fetch_width"]
    decode_width: int | None = _PIPELINE["decode_width"]
    rename_width: int | None = _PIPELINE["rename_width"]
    issue_width: int | None = _PIPELINE["issue_width"]
    commit_width: int | None = _PIPELINE["commit_width"]
    writeback_width: int | None = _PIPELINE["writeback_width"]
    trap_latency: int = _PIPELINE["trap_latency"]
    redirect_latency: int | None = _PIPELINE["redirect_latency"]
    fetch_decode_latency: int = _PIPELINE["fetch_decode_latency"]
    decode_rename_latency: int = _PIPELINE["decode_rename_latency"]
    rename_issue_latency: int = _PIPELINE["rename_issue_latency"]
    csr_squash: str | None = _PIPELINE["csr_squash"]
    fence_squash: bool = _PIPELINE["fence_squash"]
    store_forward_latency: int | None = _PIPELINE["store_forward_latency"]
    branch_predictor: _AnyBranchPredictor = _DEFAULT_BRANCH_PREDICTOR
    backend: _AnyBackend = _DEFAULT_BACKEND
    mem_dep_predictor: _AnyMemDepPredictor = _DEFAULT_MEM_DEP_PREDICTOR
    btb_size: int = _PIPELINE["btb_size"]
    btb_ways: int = _PIPELINE["btb_ways"]
    ras_size: int = _PIPELINE["ras_size"]
    # Caches (None = disabled)
    l1i: Cache | None = _DEFAULT_L1I
    l1d: Cache | None = _DEFAULT_L1D
    l2: Cache | None = _DEFAULT_L2
    l3: Cache | None = _DEFAULT_L3
    inclusion_policy: _AnyInclusionPolicy = _DEFAULT_INCLUSION_POLICY
    wcb_entries: int = _CACHES["wcb_entries"]
    load_prefetcher: LoadPrefetcher.Stride | None = _DEFAULT_LOAD_PREFETCHER
    store_prefetcher: StorePrefetcher.Stream | None = _DEFAULT_STORE_PREFETCHER
    # Memory
    ram_size: str | int = _MEMORY["ram_size"]
    memory_controller: _AnyMemoryController = _DEFAULT_MEMORY_CONTROLLER
    tlb_size: int = _MEMORY["tlb_size"]
    tlb_ways: int = _MEMORY["tlb_ways"]
    l2_tlb_size: int = _MEMORY["l2_tlb_size"]
    l2_tlb_ways: int = _MEMORY["l2_tlb_ways"]
    l2_tlb_latency: int = _MEMORY["l2_tlb_latency"]
    misaligned_access_trap: bool = _MEMORY["misaligned_access_trap"]
    paging_mode_max: str = _MEMORY["paging_mode_max"]
    # ISA extensions
    isa: str | None = _ISA["string"]
    svadu: bool = _ISA["svadu"]
    # Vector ISA
    vlen: int = _PIPELINE["vlen"]
    num_vec_lanes: int | None = _PIPELINE["num_vec_lanes"]
    vector_mem_width: int | None = _PIPELINE["vector_mem_width"]
    # General
    trace: bool = _GENERAL["trace_instructions"]
    page_mode: str = _GENERAL["page_mode"]
    # System (advanced)
    ram_base: int = _SYSTEM["ram_base"]
    uart_base: int = _SYSTEM["uart_base"]
    disk_base: int = _SYSTEM["disk_base"]
    clint_base: int = _SYSTEM["clint_base"]
    syscon_base: int = _SYSTEM["syscon_base"]
    kernel_offset: int = _SYSTEM["kernel_offset"]
    bus_width: int = _SYSTEM["bus_width"]
    bus_latency: int = _SYSTEM["bus_latency"]
    timer: str = _SYSTEM["timer"]
    timebase_hz: int = _SYSTEM["timebase_hz"]
    cpu_clock_mhz: int = _SYSTEM["cpu_clock_mhz"]
    device_latency_ns: int = _SYSTEM["device_latency_ns"]
    device_latency_ns_overrides: dict[str, int] | None = None
    rtc_epoch_seconds: int = _SYSTEM["rtc_epoch_seconds"]
    console: ConsoleMode | str = _SYSTEM["console"]
    hart_count: int = _SYSTEM["hart_count"]
    coherence: Coherence = _DEFAULT_COHERENCE

    def __post_init__(self) -> None:
        # Each Config owns its components, so editing one leaves the
        # defaults and any Config it was made from alone.
        for name in _COMPONENT_FIELDS:
            setattr(self, name, copy.deepcopy(getattr(self, name)))
        self.ram_size = _parse_size(self.ram_size)
        self.device_latency_ns_overrides = dict(self.device_latency_ns_overrides or {})
        self.console = ConsoleMode(self.console)
        check_config(self.to_dict())

    def isa_string(self) -> str:
        """The ISA string the hart implements, as the device tree advertises
        it: ``isa`` resolved, or everything the simulator has when ``isa`` is
        ``None``. Raises ``ValueError`` for an ``isa`` the simulator cannot
        implement."""
        from .._core import isa_string

        return isa_string(self.to_dict())

    def to_dict(self) -> dict[str, Any]:
        """Produce the nested dict expected by the Rust backend."""
        return _config_to_dict_impl(self)

    def replace(self, **kwargs: Any) -> Config:
        """Return a new Config with the given fields overridden.

        Example::

            base = Config(width=4, branch_predictor=BranchPredictor.Tage())
            wide = base.replace(width=8)
            ooo  = base.replace(backend=Backend.OutOfOrder(rob_size=128))
        """
        return dataclasses.replace(self, **kwargs)

    def __repr__(self) -> str:
        parts = [
            f"width={self.width}",
            f"branch_predictor={self.branch_predictor!r}",
            f"backend={self.backend!r}",
        ]
        if self.l1i is not None:
            parts.append(f"l1i={self.l1i!r}")
        if self.l1d is not None:
            parts.append(f"l1d={self.l1d!r}")
        if self.l2 is not None:
            parts.append(f"l2={self.l2!r}")
        if self.l3 is not None:
            parts.append(f"l3={self.l3!r}")
        if self.load_prefetcher is not None:
            parts.append(f"load_prefetcher={self.load_prefetcher!r}")
        if self.store_prefetcher is not None:
            parts.append(f"store_prefetcher={self.store_prefetcher!r}")
        return f"Config({', '.join(parts)})"


def _config_to_dict(config: Config | dict[str, Any]) -> dict[str, Any]:
    """The nested dict the core reads: a `Config`'s, or a dict as given."""
    if isinstance(config, Config):
        return config.to_dict()
    if isinstance(config, dict):
        return config
    raise TypeError(f"config must be a Config or a dict, not {type(config).__name__}")


def load_config(path: str) -> Config:
    """Load a `Config` from a Python file.

    The module is imported and the first of these is used as the entry point:
    a function named after the file, a ``config`` variable, or a ``get_config``
    function. Raises ``TypeError`` when it gives something other than a
    `Config`.
    """
    import importlib.util
    import os

    spec = importlib.util.spec_from_file_location("custom_config", path)
    if not (spec and spec.loader):
        raise ImportError(f"could not load config file: {path}")

    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    name = os.path.splitext(os.path.basename(path))[0]
    attribute = next(
        (a for a in (name, "config", "get_config") if hasattr(mod, a)), None
    )
    if attribute is None:
        raise AttributeError(
            f"could not find config entry point in {path}. "
            f"Expected function '{name}' or 'get_config' or variable 'config'."
        )
    entry = getattr(mod, attribute)
    config = entry() if callable(entry) else entry
    if not isinstance(config, Config):
        raise TypeError(
            f"{path}: {attribute} gave a {type(config).__name__}, not a Config"
        )
    return config
