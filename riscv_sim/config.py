"""
Python-first configuration for the RISC-V simulator.

This module provides:
1. **Config dataclasses:** `GeneralConfig`, `SystemConfig`, `MemoryConfig`, `CacheConfig`, `PipelineConfig`, and hierarchy types.
2. **SimConfig:** Full simulator config with `to_dict()` for the Rust backend; use `SimConfig.default()` or `SimConfig.minimal()` as base.
3. **config_to_dict:** Normalizes `SimConfig` or a plain dict for the backend.

Enum types (`MemoryControllerT`, etc.) use string literals that must match Rust serde (PascalCase/snake_case).
Build your machine model in scripts—see scripts/p550/config.py and scripts/m1/config.py for examples.

Example:
    from riscv_sim import SimConfig, Environment, run_experiment

    config = SimConfig.default()
    config.cache.l1_d.enabled = True
    config.cache.l1_d.size_bytes = 32768
    env = Environment(binary="bench.bin", config=config)
    result = run_experiment(env)
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Literal, Optional

MemoryControllerT = Literal["Simple", "Dram"]
ReplacementPolicyT = Literal["LRU", "PLRU", "FIFO", "Random", "MRU"]
PrefetcherT = Literal["None", "NextLine", "Stride", "Stream", "Tagged"]
BranchPredictorT = Literal["Static", "GShare", "Perceptron", "TAGE", "Tournament"]


@dataclass
class GeneralConfig:
    """General simulation settings (tracing, start PC, direct mode, initial stack pointer)."""
    trace_instructions: bool = False
    start_pc: int = 0x8000_0000
    direct_mode: bool = True
    initial_sp: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        d: Dict[str, Any] = {
            "trace_instructions": self.trace_instructions,
            "start_pc": self.start_pc,
            "direct_mode": self.direct_mode,
        }
        if self.initial_sp is not None:
            d["initial_sp"] = self.initial_sp
        return d


@dataclass
class SystemConfig:
    """System memory map and bus parameters."""
    ram_base: int = 0x8000_0000
    uart_base: int = 0x1000_0000
    disk_base: int = 0x9000_0000
    clint_base: int = 0x0200_0000
    syscon_base: int = 0x0010_0000
    kernel_offset: int = 0x0020_0000
    bus_width: int = 8
    bus_latency: int = 4
    clint_divider: int = 10
    uart_to_stderr: bool = False

    def to_dict(self) -> Dict[str, Any]:
        return {
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


@dataclass
class MemoryConfig:
    """Main memory configuration (RAM size, controller type, DRAM timing, TLB size)."""
    ram_size: int = 0x1000_0000
    controller: MemoryControllerT = "Simple"
    t_cas: int = 14
    t_ras: int = 14
    t_pre: int = 14
    row_miss_latency: int = 120
    tlb_size: int = 32

    def to_dict(self) -> Dict[str, Any]:
        return {
            "ram_size": self.ram_size,
            "controller": self.controller,
            "t_cas": self.t_cas,
            "t_ras": self.t_ras,
            "t_pre": self.t_pre,
            "row_miss_latency": self.row_miss_latency,
            "tlb_size": self.tlb_size,
        }


@dataclass
class CacheConfig:
    """Single cache level (L1-I, L1-D, L2, L3)."""
    enabled: bool = False
    size_bytes: int = 4096
    line_bytes: int = 64
    ways: int = 1
    policy: ReplacementPolicyT = "LRU"
    latency: int = 1
    prefetcher: PrefetcherT = "None"
    prefetch_table_size: int = 0
    prefetch_degree: int = 0

    def to_dict(self) -> Dict[str, Any]:
        return {
            "enabled": self.enabled,
            "size_bytes": self.size_bytes,
            "line_bytes": self.line_bytes,
            "ways": self.ways,
            "policy": self.policy,
            "latency": self.latency,
            "prefetcher": self.prefetcher,
            "prefetch_table_size": self.prefetch_table_size,
            "prefetch_degree": self.prefetch_degree,
        }


@dataclass
class TageConfig:
    """TAGE branch predictor parameters."""
    num_banks: int = 4
    table_size: int = 2048
    loop_table_size: int = 256
    reset_interval: int = 2000
    history_lengths: List[int] = field(default_factory=lambda: [5, 15, 44, 130])
    tag_widths: List[int] = field(default_factory=lambda: [9, 9, 10, 10])

    def to_dict(self) -> Dict[str, Any]:
        return {
            "num_banks": self.num_banks,
            "table_size": self.table_size,
            "loop_table_size": self.loop_table_size,
            "reset_interval": self.reset_interval,
            "history_lengths": self.history_lengths,
            "tag_widths": self.tag_widths,
        }


@dataclass
class PerceptronConfig:
    """Perceptron branch predictor parameters."""
    history_length: int = 32
    table_bits: int = 10

    def to_dict(self) -> Dict[str, Any]:
        return {
            "history_length": self.history_length,
            "table_bits": self.table_bits,
        }


@dataclass
class TournamentConfig:
    """Tournament branch predictor parameters."""
    global_size_bits: int = 12
    local_hist_bits: int = 10
    local_pred_bits: int = 10

    def to_dict(self) -> Dict[str, Any]:
        return {
            "global_size_bits": self.global_size_bits,
            "local_hist_bits": self.local_hist_bits,
            "local_pred_bits": self.local_pred_bits,
        }


@dataclass
class PipelineConfig:
    """Pipeline and branch predictor configuration."""
    width: int = 1
    branch_predictor: BranchPredictorT = "Static"
    btb_size: int = 256
    ras_size: int = 8
    tage: TageConfig = field(default_factory=TageConfig)
    perceptron: PerceptronConfig = field(default_factory=PerceptronConfig)
    tournament: TournamentConfig = field(default_factory=TournamentConfig)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "width": self.width,
            "branch_predictor": self.branch_predictor,
            "btb_size": self.btb_size,
            "ras_size": self.ras_size,
            "tage": self.tage.to_dict(),
            "perceptron": self.perceptron.to_dict(),
            "tournament": self.tournament.to_dict(),
        }


@dataclass
class CacheHierarchyConfig:
    """L1-I, L1-D, L2, L3 cache configuration."""
    l1_i: CacheConfig = field(default_factory=CacheConfig)
    l1_d: CacheConfig = field(default_factory=CacheConfig)
    l2: CacheConfig = field(default_factory=CacheConfig)
    l3: CacheConfig = field(default_factory=CacheConfig)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "l1_i": self.l1_i.to_dict(),
            "l1_d": self.l1_d.to_dict(),
            "l2": self.l2.to_dict(),
            "l3": self.l3.to_dict(),
        }


@dataclass
class SimConfig:
    """
    Full simulator configuration. The library only provides base configs;
    you build your machine model in scripts (see scripts/p550/config.py, scripts/m1/config.py).
    """

    general: GeneralConfig = field(default_factory=GeneralConfig)
    system: SystemConfig = field(default_factory=SystemConfig)
    memory: MemoryConfig = field(default_factory=MemoryConfig)
    cache: CacheHierarchyConfig = field(default_factory=CacheHierarchyConfig)
    pipeline: PipelineConfig = field(default_factory=PipelineConfig)

    def to_dict(self) -> Dict[str, Any]:
        """Produce the nested dict expected by the Rust backend (JSON round-trip)."""
        return {
            "general": self.general.to_dict(),
            "system": self.system.to_dict(),
            "memory": self.memory.to_dict(),
            "cache": self.cache.to_dict(),
            "pipeline": self.pipeline.to_dict(),
        }

    @classmethod
    def minimal(cls) -> SimConfig:
        """Minimal config: no caches, static BP, 1-wide. Fast for debugging."""
        c = cls()
        c.memory.ram_size = 128 * 1024 * 1024
        c.cache.l1_i.enabled = False
        c.cache.l1_d.enabled = False
        c.cache.l2.enabled = False
        c.cache.l3.enabled = False
        c.pipeline.width = 1
        c.pipeline.branch_predictor = "Static"
        return c

    @classmethod
    def default(cls) -> SimConfig:
        """Base config: 1-wide, static BP, caches disabled. Start here and set up your own."""
        c = cls()
        c.memory.ram_size = 0x1000_0000
        return c


def config_to_dict(config: SimConfig | Dict[str, Any]) -> Dict[str, Any]:
    """
    Normalize config to a dict for the Rust backend.

    Accepts SimConfig (uses .to_dict()) or a plain dict (returned as-is).
    """
    if hasattr(config, "to_dict") and callable(getattr(config, "to_dict")):
        return config.to_dict()
    if isinstance(config, dict):
        return config
    raise TypeError("config must be SimConfig or dict")
