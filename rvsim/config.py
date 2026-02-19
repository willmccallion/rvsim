"""
Flat simulator configuration.

A single ``Config`` class replaces the old nested dataclass hierarchy.
All parameters live at the top level; ``to_dict()`` assembles the nested
dict that the Rust backend expects.
"""

from __future__ import annotations

from typing import Any, Dict, Optional

from .types import (
    _parse_size,
    BranchPredictor,
    ReplacementPolicy,
    Prefetcher,
    MemoryController,
    Backend,
    Cache,
    _DISABLED_CACHE_DICT,
    _DISABLED_CACHE_DICT_ZERO,
)


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
        width: int = 1,
        branch_predictor=BranchPredictor.TAGE(),
        backend=None,
        btb_size: int = 4096,
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
        # Memory
        ram_size="256MB",
        memory_controller=None,
        tlb_size: int = 32,
        # General
        trace: bool = False,
        start_pc: int = 0x8000_0000,
        direct_mode: bool = True,
        initial_sp: Optional[int] = None,
        # System (advanced)
        ram_base: int = 0x8000_0000,
        uart_base: int = 0x1000_0000,
        disk_base: int = 0x9000_0000,
        clint_base: int = 0x0200_0000,
        syscon_base: int = 0x0010_0000,
        kernel_offset: int = 0x0020_0000,
        bus_width: int = 8,
        bus_latency: int = 4,
        clint_divider: int = 10,
        uart_to_stderr: bool = False,
    ):
        # Pipeline
        self.width = width
        self.branch_predictor = branch_predictor
        self.backend = backend if backend is not None else Backend.InOrder()
        self.btb_size = btb_size
        self.ras_size = ras_size

        # Caches
        self.l1i = l1i
        self.l1d = l1d
        self.l2 = l2
        self.l3 = l3

        # Memory
        self.ram_size = _parse_size(ram_size)
        self.memory_controller = (
            memory_controller
            if memory_controller is not None
            else MemoryController.Simple()
        )
        self.tlb_size = tlb_size

        # General
        self.trace = trace
        self.start_pc = start_pc
        self.direct_mode = direct_mode
        self.initial_sp = initial_sp

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
        self.uart_to_stderr = uart_to_stderr

    def to_dict(self) -> Dict[str, Any]:
        """Produce the nested dict expected by the Rust backend."""
        # General
        general: Dict[str, Any] = {
            "trace_instructions": self.trace,
            "start_pc": self.start_pc,
            "direct_mode": self.direct_mode,
        }
        if self.initial_sp is not None:
            general["initial_sp"] = self.initial_sp

        # System
        system = {
            "ram_base": self.ram_base,
            "uart_base": self.uart_base,
            "disk_base": self.disk_base,
            "clint_base": self.clint_base,
            "syscon_base": self.syscon_base,
            "kernel_offset": self.kernel_offset,
            "bus_width": self.bus_width,
            "bus_latency": self.bus_latency,
            "clint_divider": self.clint_divider,
            "uart_to_stderr": self.uart_to_stderr,
        }

        # Memory — merge controller-specific params
        mc = self.memory_controller
        memory: Dict[str, Any] = {
            "ram_size": self.ram_size,
            "controller": mc._to_dict_value(),
            "tlb_size": self.tlb_size,
        }
        # Always emit DRAM timing keys (Rust expects them)
        if isinstance(mc, MemoryController.DRAM):
            memory.update(mc._sub_dict())
        else:
            memory["t_cas"] = 14
            memory["t_ras"] = 14
            memory["t_pre"] = 14
            memory["row_miss_latency"] = 120

        # Caches
        cache = {
            "l1_i": (
                self.l1i._to_cache_dict()
                if self.l1i is not None
                else _DISABLED_CACHE_DICT
            ),
            "l1_d": (
                self.l1d._to_cache_dict()
                if self.l1d is not None
                else _DISABLED_CACHE_DICT
            ),
            "l2": (
                self.l2._to_cache_dict()
                if self.l2 is not None
                else _DISABLED_CACHE_DICT_ZERO
            ),
            "l3": (
                self.l3._to_cache_dict()
                if self.l3 is not None
                else _DISABLED_CACHE_DICT_ZERO
            ),
        }

        # Pipeline — always emit all three BP sub-configs with defaults
        bp = self.branch_predictor
        tage_dict = BranchPredictor.TAGE()._sub_dict()
        perceptron_dict = BranchPredictor.Perceptron()._sub_dict()
        tournament_dict = BranchPredictor.Tournament()._sub_dict()

        if isinstance(bp, BranchPredictor.TAGE):
            tage_dict = bp._sub_dict()
        elif isinstance(bp, BranchPredictor.Perceptron):
            perceptron_dict = bp._sub_dict()
        elif isinstance(bp, BranchPredictor.Tournament):
            tournament_dict = bp._sub_dict()

        pipeline = {
            "width": self.width,
            "branch_predictor": bp._to_dict_value(),
            "btb_size": self.btb_size,
            "ras_size": self.ras_size,
            "backend": self.backend._to_dict_value(),
            "rob_size": self.backend._rob_size(),
            "store_buffer_size": self.backend._store_buffer_size(),
            "tage": tage_dict,
            "perceptron": perceptron_dict,
            "tournament": tournament_dict,
        }

        return {
            "general": general,
            "system": system,
            "memory": memory,
            "cache": cache,
            "pipeline": pipeline,
        }

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


def _config_to_dict(config) -> Dict[str, Any]:
    """Normalize config to a dict for the Rust backend. Accepts Config or plain dict."""
    if hasattr(config, "to_dict") and callable(getattr(config, "to_dict")):
        return config.to_dict()
    if isinstance(config, dict):
        return config
    raise TypeError("config must be Config or dict")
