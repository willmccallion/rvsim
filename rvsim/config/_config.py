"""The flat ``Config``: every parameter at the top level, assembled into
the nested dict the Rust core expects."""

from __future__ import annotations

import copy
import inspect
from typing import Any

from ._core_defaults import MACHINE
from ._serialize import _config_to_dict_impl
from ._units import _parse_size
from .backend import Backend, _backend_from
from .branch import (
    BranchPredictor,
    MemDepPredictor,
    _branch_predictor_from,
    _mem_dep_predictor_from,
)
from .coherence import Coherence
from .memory import (
    Cache,
    LoadPrefetcher,
    StorePrefetcher,
    _cache_from,
    _controller_from,
    _load_prefetcher_from,
    _store_prefetcher_from,
)

_PAGING_MODES = ("Bare", "Sv39", "Sv48", "Sv57")


def _validate_paging_mode(value: str) -> str:
    """Check a paging-mode cap: one of the core's paging modes."""
    if not isinstance(value, str):
        raise TypeError(f"paging_mode_max must be a string, got {type(value).__name__}")
    if value not in _PAGING_MODES:
        raise ValueError(f"paging_mode_max={value!r} not in {_PAGING_MODES}")
    return value


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


class Config:
    """
    Full simulator configuration with flat parameter access.

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

    def __init__(
        self,
        # Pipeline
        width: int = _PIPELINE["width"],
        fetch_width: int | None = _PIPELINE["fetch_width"],
        decode_width: int | None = _PIPELINE["decode_width"],
        rename_width: int | None = _PIPELINE["rename_width"],
        issue_width: int | None = _PIPELINE["issue_width"],
        commit_width: int | None = _PIPELINE["commit_width"],
        writeback_width: int | None = _PIPELINE["writeback_width"],
        trap_latency: int = _PIPELINE["trap_latency"],
        redirect_latency: int | None = _PIPELINE["redirect_latency"],
        fetch_decode_latency: int = _PIPELINE["fetch_decode_latency"],
        decode_rename_latency: int = _PIPELINE["decode_rename_latency"],
        rename_issue_latency: int = _PIPELINE["rename_issue_latency"],
        csr_squash: str | None = _PIPELINE["csr_squash"],
        fence_squash: bool = _PIPELINE["fence_squash"],
        store_forward_latency: int | None = _PIPELINE["store_forward_latency"],
        branch_predictor: BranchPredictor.Static
        | BranchPredictor.GShare
        | BranchPredictor.Bht
        | BranchPredictor.Tage
        | BranchPredictor.ScLTage
        | BranchPredictor.Perceptron
        | BranchPredictor.Tournament = _DEFAULT_BRANCH_PREDICTOR,
        backend: Backend.InOrder | Backend.OutOfOrder = _DEFAULT_BACKEND,
        mem_dep_predictor: MemDepPredictor.Blind
        | MemDepPredictor.StoreSet = _DEFAULT_MEM_DEP_PREDICTOR,
        btb_size: int = _PIPELINE["btb_size"],
        btb_ways: int = _PIPELINE["btb_ways"],
        ras_size: int = _PIPELINE["ras_size"],
        # Caches (None = disabled)
        l1i: Cache | None = _DEFAULT_L1I,
        l1d: Cache | None = _DEFAULT_L1D,
        l2: Cache | None = _DEFAULT_L2,
        l3: Cache | None = _DEFAULT_L3,
        inclusion_policy: Any = _DEFAULT_INCLUSION_POLICY,
        wcb_entries: int = _CACHES["wcb_entries"],
        load_prefetcher: LoadPrefetcher.Stride | None = _DEFAULT_LOAD_PREFETCHER,
        store_prefetcher: StorePrefetcher.Stream | None = _DEFAULT_STORE_PREFETCHER,
        # Memory
        ram_size: str | int = _MEMORY["ram_size"],
        memory_controller: Any = _DEFAULT_MEMORY_CONTROLLER,
        tlb_size: int = _MEMORY["tlb_size"],
        tlb_ways: int = _MEMORY["tlb_ways"],
        l2_tlb_size: int = _MEMORY["l2_tlb_size"],
        l2_tlb_ways: int = _MEMORY["l2_tlb_ways"],
        l2_tlb_latency: int = _MEMORY["l2_tlb_latency"],
        misaligned_access_trap: bool = _MEMORY["misaligned_access_trap"],
        paging_mode_max: str = _MEMORY["paging_mode_max"],
        # ISA extensions
        isa: str | None = _ISA["string"],
        svadu: bool = _ISA["svadu"],
        # Vector ISA
        vlen: int = _PIPELINE["vlen"],
        num_vec_lanes: int | None = _PIPELINE["num_vec_lanes"],
        vector_mem_width: int | None = _PIPELINE["vector_mem_width"],
        # General
        trace: bool = _GENERAL["trace_instructions"],
        page_mode: str = _GENERAL["page_mode"],
        clock_hz: int = _GENERAL["clock_hz"],
        # System (advanced)
        ram_base: int = _SYSTEM["ram_base"],
        uart_base: int = _SYSTEM["uart_base"],
        disk_base: int = _SYSTEM["disk_base"],
        clint_base: int = _SYSTEM["clint_base"],
        syscon_base: int = _SYSTEM["syscon_base"],
        kernel_offset: int = _SYSTEM["kernel_offset"],
        bus_width: int = _SYSTEM["bus_width"],
        bus_latency: int = _SYSTEM["bus_latency"],
        clint_divider: int = _SYSTEM["clint_divider"],
        cpu_clock_mhz: int = _SYSTEM["cpu_clock_mhz"],
        device_latency_ns: int = _SYSTEM["device_latency_ns"],
        device_latency_ns_overrides: dict[str, int] | None = None,
        rtc_epoch_seconds: int = _SYSTEM["rtc_epoch_seconds"],
        uart_to_stderr: bool = False,
        uart_quiet: bool = False,
        console: str | None = None,
        hart_count: int = _SYSTEM["hart_count"],
        coherence: Coherence | None = None,
    ):
        # Pipeline
        self.width = width
        self.fetch_width = fetch_width
        self.decode_width = decode_width
        self.rename_width = rename_width
        self.issue_width = issue_width
        self.commit_width = commit_width
        self.writeback_width = writeback_width
        self.trap_latency = trap_latency
        self.redirect_latency = redirect_latency
        self.fetch_decode_latency = fetch_decode_latency
        self.decode_rename_latency = decode_rename_latency
        self.rename_issue_latency = rename_issue_latency
        if csr_squash not in (None, "EveryAccess", "AffectingWrites", "Never"):
            raise ValueError(
                f"csr_squash must be 'EveryAccess', 'AffectingWrites' or 'Never', not {csr_squash!r}"
            )
        self.csr_squash = csr_squash
        self.fence_squash = fence_squash
        self.store_forward_latency = store_forward_latency
        self.branch_predictor = copy.deepcopy(branch_predictor)
        self.backend = (
            copy.deepcopy(backend) if backend is not None else Backend.InOrder()
        )
        if csr_squash == "Never" and isinstance(self.backend, Backend.InOrder):
            raise ValueError(
                "csr_squash 'Never' needs the out-of-order backend's rename hold; "
                "use 'EveryAccess' or 'AffectingWrites'"
            )
        self.mem_dep_predictor = copy.deepcopy(mem_dep_predictor)
        self.btb_size = btb_size
        self.btb_ways = btb_ways
        self.ras_size = ras_size

        # Caches
        self.l1i = copy.deepcopy(l1i)
        self.l1d = copy.deepcopy(l1d)
        self.l2 = copy.deepcopy(l2)
        self.l3 = copy.deepcopy(l3)
        self.inclusion_policy = copy.deepcopy(inclusion_policy)
        self.wcb_entries = wcb_entries
        self.load_prefetcher = copy.deepcopy(load_prefetcher)
        self.store_prefetcher = copy.deepcopy(store_prefetcher)

        # Memory
        self.ram_size = _parse_size(ram_size)
        self.memory_controller = copy.deepcopy(
            memory_controller
            if memory_controller is not None
            else _DEFAULT_MEMORY_CONTROLLER
        )
        self.tlb_size = tlb_size
        self.tlb_ways = tlb_ways
        self.l2_tlb_size = l2_tlb_size
        self.l2_tlb_ways = l2_tlb_ways
        self.l2_tlb_latency = l2_tlb_latency
        self.misaligned_access_trap = misaligned_access_trap
        self.paging_mode_max = _validate_paging_mode(paging_mode_max)

        # ISA extensions
        self.isa = isa
        self.svadu = svadu

        # Vector ISA
        self.vlen = vlen
        self.num_vec_lanes = num_vec_lanes
        self.vector_mem_width = vector_mem_width

        # General
        self.trace = trace
        if page_mode not in ("Sv39", "Sv48", "Sv57"):
            raise ValueError(
                f"page_mode must be 'Sv39', 'Sv48' or 'Sv57', not {page_mode!r}"
            )
        self.page_mode = page_mode
        if clock_hz <= 0:
            raise ValueError(f"clock_hz must be positive, not {clock_hz}")
        self.clock_hz = clock_hz

        # System
        self.ram_base = ram_base
        self.uart_base = uart_base
        self.disk_base = disk_base
        self.clint_base = clint_base
        self.syscon_base = syscon_base
        self.kernel_offset = kernel_offset
        self.bus_width = bus_width
        self.bus_latency = bus_latency
        self.clint_divider = clint_divider
        self.cpu_clock_mhz = cpu_clock_mhz
        self.device_latency_ns = device_latency_ns
        self.device_latency_ns_overrides = dict(device_latency_ns_overrides or {})
        self.rtc_epoch_seconds = rtc_epoch_seconds
        self.uart_to_stderr = uart_to_stderr
        self.uart_quiet = uart_quiet
        if console is not None and console not in _CONSOLES:
            raise ValueError(f"console must be one of {_CONSOLES}, got {console!r}")
        self.console = console
        self.hart_count = hart_count
        self.coherence = (
            copy.deepcopy(coherence) if coherence is not None else Coherence()
        )

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

    def replace(self, **kwargs) -> Config:
        """Return a new Config with the given fields overridden.

        Example::

            base = Config(width=4, branch_predictor=BranchPredictor.Tage())
            wide = base.replace(width=8)
            ooo  = base.replace(backend=Backend.OutOfOrder(rob_size=128))
        """
        fields = {
            name: getattr(self, name)
            for name in inspect.signature(Config.__init__).parameters
            if name != "self"
        }
        unknown = set(kwargs) - set(fields)
        if unknown:
            raise TypeError(f"Config.replace() got unexpected fields: {unknown}")
        fields.update(kwargs)
        return Config(**fields)  # type: ignore[arg-type]

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


_CONSOLES = ("Stdout", "Stderr", "Quiet", "Captured")


def _config_to_dict(config) -> dict[str, Any]:
    """Normalize config to a dict for the Rust backend. Accepts Config or plain dict."""
    if hasattr(config, "to_dict") and callable(config.to_dict):
        return config.to_dict()
    if isinstance(config, dict):
        return config
    raise TypeError("config must be Config or dict")


def load_config(path: str) -> Config:
    """Load a `Config` from a Python file.

    The module is imported and the first of these is used as the entry point:
    a function named after the file, a ``config`` variable, or a ``get_config``
    function.
    """
    import importlib.util
    import os

    if not os.path.exists(path) and os.path.exists(os.path.join(os.getcwd(), path)):
        path = os.path.join(os.getcwd(), path)

    spec = importlib.util.spec_from_file_location("custom_config", path)
    if not (spec and spec.loader):
        raise ImportError(f"could not load config file: {path}")

    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    name = os.path.splitext(os.path.basename(path))[0]
    if hasattr(mod, name):
        entry = getattr(mod, name)
        return entry() if callable(entry) else entry
    if hasattr(mod, "config"):
        entry = mod.config
        return entry() if callable(entry) else entry
    if hasattr(mod, "get_config"):
        return mod.get_config()

    raise AttributeError(
        f"could not find config entry point in {path}. "
        f"Expected function '{name}' or 'get_config' or variable 'config'."
    )
