"""The flat ``Config``: every parameter at the top level, assembled into
the nested dict the Rust core expects."""

from __future__ import annotations


import inspect
from typing import Any, Dict, Optional

from ._serialize import _config_to_dict_impl
from ._units import _parse_size
from .backend import Backend
from .branch import BranchPredictor, MemDepPredictor
from .coherence import Coherence
from .memory import Cache, MemoryController, Prefetcher


_PAGING_MODES = ("bare", "sv39", "sv48", "sv57")


def _validate_paging_mode(value: str) -> str:
    """Normalize and validate a paging-mode-cap string for the Rust deserializer."""
    if not isinstance(value, str):
        raise TypeError(f"paging_mode_max must be a string, got {type(value).__name__}")
    normalized = value.lower()
    if normalized not in _PAGING_MODES:
        raise ValueError(f"paging_mode_max={value!r} not in {_PAGING_MODES}")
    return normalized


class Config:
    """
    Full simulator configuration with flat parameter access.

    Example::

        from rvsim import Config, Cache, BranchPredictor, Prefetcher

        cfg = Config(
            width=4,
            branch_predictor=BranchPredictor.TAGE(),
            l1i=Cache("128KB", ways=8, prefetcher=Prefetcher.NextLine(degree=2)),
            l1d=Cache("128KB", ways=8, prefetcher=Prefetcher.Stride(degree=2, table_size=128)),
            l2=Cache("4MB", ways=16, latency=12),
        )
    """

    def __init__(
        self,
        # Pipeline
        width: int = 4,
        fetch_width: Optional[int] = None,
        decode_width: Optional[int] = None,
        rename_width: Optional[int] = None,
        issue_width: Optional[int] = None,
        commit_width: Optional[int] = None,
        writeback_width: Optional[int] = None,
        trap_latency: int = 13,
        redirect_latency: Optional[int] = None,
        branch_predictor: "BranchPredictor.Static | BranchPredictor.GShare | BranchPredictor.TAGE | BranchPredictor.Perceptron | BranchPredictor.Tournament" = BranchPredictor.TAGE(),
        backend: "Backend.InOrder | Backend.OutOfOrder" = Backend.OutOfOrder(),
        mem_dep_predictor: "MemDepPredictor.Blind | MemDepPredictor.StoreSet" = MemDepPredictor.StoreSet(),
        btb_size: int = 4096,
        btb_ways: int = 4,
        ras_size: int = 32,
        # Caches (None = disabled)
        l1i=Cache("32KB", ways=4, latency=1, prefetcher=Prefetcher.NextLine(degree=1)),
        l1d=Cache(
            "32KB",
            ways=4,
            latency=1,
            prefetcher=Prefetcher.Stride(degree=1, table_size=64),
        ),
        l2=Cache("256KB", ways=8, latency=10),
        l3: Optional[Cache] = None,
        inclusion_policy: Any = Cache.NINE(),
        wcb_entries: int = 0,
        # Memory
        ram_size="256MB",
        memory_controller=None,
        tlb_size: int = 64,
        tlb_ways: int = 0,
        l2_tlb_size: int = 0,
        l2_tlb_ways: int = 4,
        l2_tlb_latency: int = 4,
        misaligned_access_trap: bool = False,
        paging_mode_max: str = "sv57",
        # ISA extensions
        svadu: bool = False,
        # Vector ISA
        vlen: int = 128,
        num_vec_lanes: Optional[int] = None,
        vector_mem_width: Optional[int] = None,
        # General
        trace: bool = False,
        initial_sp: Optional[int] = None,
        # System (advanced)
        ram_base: int = 0x8000_0000,
        uart_base: int = 0x1000_0000,
        disk_base: int = 0x9000_0000,
        clint_base: int = 0x0200_0000,
        syscon_base: int = 0x0010_0000,
        sim_control_base: int = 0x0010_2000,
        kernel_offset: int = 0x0020_0000,
        bus_width: int = 8,
        bus_latency: int = 4,
        clint_divider: int = 10,
        cpu_clock_mhz: int = 2400,
        device_latency_ns: int = 100,
        device_latency_ns_overrides: Optional[Dict[str, int]] = None,
        rtc_epoch_seconds: int = 1_767_225_600,
        uart_to_stderr: bool = False,
        uart_quiet: bool = False,
        console: Optional[str] = None,
        hart_count: int = 1,
        coherence: Optional[Coherence] = None,
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
        self.branch_predictor = branch_predictor
        self.backend = backend if backend is not None else Backend.InOrder()
        self.mem_dep_predictor = mem_dep_predictor
        self.btb_size = btb_size
        self.btb_ways = btb_ways
        self.ras_size = ras_size

        # Caches
        self.l1i = l1i
        self.l1d = l1d
        self.l2 = l2
        self.l3 = l3
        self.inclusion_policy = inclusion_policy
        self.wcb_entries = wcb_entries

        # Memory
        self.ram_size = _parse_size(ram_size)
        self.memory_controller = (
            memory_controller
            if memory_controller is not None
            else MemoryController.Simple()
        )
        self.tlb_size = tlb_size
        self.tlb_ways = tlb_ways
        self.l2_tlb_size = l2_tlb_size
        self.l2_tlb_ways = l2_tlb_ways
        self.l2_tlb_latency = l2_tlb_latency
        self.misaligned_access_trap = misaligned_access_trap
        self.paging_mode_max = _validate_paging_mode(paging_mode_max)

        # ISA extensions
        self.svadu = svadu

        # Vector ISA
        self.vlen = vlen
        self.num_vec_lanes = num_vec_lanes
        self.vector_mem_width = vector_mem_width

        # General
        self.trace = trace
        self.initial_sp = initial_sp

        # System
        self.ram_base = ram_base
        self.uart_base = uart_base
        self.disk_base = disk_base
        self.clint_base = clint_base
        self.syscon_base = syscon_base
        self.sim_control_base = sim_control_base
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
        self.coherence = coherence if coherence is not None else Coherence()

    def to_dict(self) -> Dict[str, Any]:
        """Produce the nested dict expected by the Rust backend."""
        return _config_to_dict_impl(self)

    def replace(self, **kwargs) -> "Config":
        """Return a new Config with the given fields overridden.

        Example::

            base = Config(width=4, branch_predictor=BranchPredictor.TAGE())
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
        return f"Config({', '.join(parts)})"


_CONSOLES = ("stdout", "stderr", "quiet", "captured")


def _config_to_dict(config) -> Dict[str, Any]:
    """Normalize config to a dict for the Rust backend. Accepts Config or plain dict."""
    if hasattr(config, "to_dict") and callable(getattr(config, "to_dict")):
        return config.to_dict()
    if isinstance(config, dict):
        return config
    raise TypeError("config must be Config or dict")


def load_config(path: str) -> "Config":
    """Load a :class:`Config` from a Python file.

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
        entry = getattr(mod, "config")
        return entry() if callable(entry) else entry
    if hasattr(mod, "get_config"):
        return getattr(mod, "get_config")()

    raise AttributeError(
        f"could not find config entry point in {path}. "
        f"Expected function '{name}' or 'get_config' or variable 'config'."
    )
