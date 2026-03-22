"""
Flat simulator configuration.

A single ``Config`` class replaces the old nested dataclass hierarchy.
All parameters live at the top level; ``to_dict()`` assembles the nested
dict that the Rust backend expects.
"""

from __future__ import annotations

from typing import Any, Dict, Optional

__all__ = ["Config"]

from .types import (
    _DISABLED_CACHE_DICT,
    _DISABLED_CACHE_DICT_ZERO,
    Backend,
    BranchPredictor,
    Cache,
    Fu,
    MemDepPredictor,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
    _parse_size,
)

_START_PC_DEFAULT = 0x8000_0000


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
        branch_predictor: "BranchPredictor.Static | BranchPredictor.GShare | BranchPredictor.TAGE | BranchPredictor.Perceptron | BranchPredictor.Tournament" = BranchPredictor.TAGE(),
        backend: "Backend.InOrder | Backend.OutOfOrder" = Backend.OutOfOrder(),
        mem_dep_predictor: "MemDepPredictor.Blind | MemDepPredictor.StoreSet" = MemDepPredictor.Blind(),
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
        tlb_size: int = 32,
        l2_tlb_size: int = 512,
        l2_tlb_ways: int = 4,
        l2_tlb_latency: int = 4,
        software_ad_bits: bool = True,
        misaligned_access_trap: bool = False,
        # General
        trace: bool = False,
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
        uart_quiet: bool = False,
    ):
        # Pipeline
        self.width = width
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
        self.l2_tlb_size = l2_tlb_size
        self.l2_tlb_ways = l2_tlb_ways
        self.l2_tlb_latency = l2_tlb_latency
        self.software_ad_bits = software_ad_bits
        self.misaligned_access_trap = misaligned_access_trap

        # General
        self.trace = trace
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
        self.uart_quiet = uart_quiet

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
        # Collect all current field values
        fields = dict(
            width=self.width,
            branch_predictor=self.branch_predictor,
            backend=self.backend,
            btb_size=self.btb_size,
            btb_ways=self.btb_ways,
            ras_size=self.ras_size,
            l1i=self.l1i,
            l1d=self.l1d,
            l2=self.l2,
            l3=self.l3,
            inclusion_policy=self.inclusion_policy,
            wcb_entries=self.wcb_entries,
            ram_size=self.ram_size,
            memory_controller=self.memory_controller,
            tlb_size=self.tlb_size,
            l2_tlb_size=self.l2_tlb_size,
            l2_tlb_ways=self.l2_tlb_ways,
            l2_tlb_latency=self.l2_tlb_latency,
            software_ad_bits=self.software_ad_bits,
            misaligned_access_trap=self.misaligned_access_trap,
            trace=self.trace,
            initial_sp=self.initial_sp,
            ram_base=self.ram_base,
            uart_base=self.uart_base,
            disk_base=self.disk_base,
            clint_base=self.clint_base,
            syscon_base=self.syscon_base,
            kernel_offset=self.kernel_offset,
            bus_width=self.bus_width,
            bus_latency=self.bus_latency,
            clint_divider=self.clint_divider,
            uart_to_stderr=self.uart_to_stderr,
            uart_quiet=self.uart_quiet,
        )
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


def _config_to_dict(config) -> Dict[str, Any]:
    """Normalize config to a dict for the Rust backend. Accepts Config or plain dict."""
    if hasattr(config, "to_dict") and callable(getattr(config, "to_dict")):
        return config.to_dict()
    if isinstance(config, dict):
        return config
    raise TypeError("config must be Config or dict")


# ── Serialization helpers (private) ──────────────────────────────────────────


def _bp_name(bp) -> str:
    """Return the branch predictor name string for the Rust backend."""
    if isinstance(bp, BranchPredictor.Static):
        return "Static"
    if isinstance(bp, BranchPredictor.GShare):
        return "GShare"
    if isinstance(bp, BranchPredictor.TAGE):
        return "TAGE"
    if isinstance(bp, BranchPredictor.Perceptron):
        return "Perceptron"
    if isinstance(bp, BranchPredictor.Tournament):
        return "Tournament"
    if isinstance(bp, BranchPredictor.ScLTage):
        return "ScLTage"
    raise TypeError(f"Unknown branch predictor type: {type(bp)}")


def _bp_sub_dict(bp) -> dict:
    """Return the branch predictor sub-config dict."""
    if isinstance(bp, (BranchPredictor.TAGE, BranchPredictor.ScLTage)):
        return {
            "num_banks": bp.num_banks,
            "table_size": bp.table_size,
            "loop_table_size": bp.loop_table_size,
            "reset_interval": bp.reset_interval,
            "history_lengths": bp.history_lengths,
            "tag_widths": bp.tag_widths,
        }
    if isinstance(bp, BranchPredictor.Perceptron):
        return {
            "history_length": bp.history_length,
            "table_bits": bp.table_bits,
        }
    if isinstance(bp, BranchPredictor.Tournament):
        return {
            "global_size_bits": bp.global_size_bits,
            "local_hist_bits": bp.local_hist_bits,
            "local_pred_bits": bp.local_pred_bits,
        }
    return {}


def _sc_sub_dict(bp) -> dict:
    """Return the SC sub-config dict for ScLTage."""
    if isinstance(bp, BranchPredictor.ScLTage):
        return {
            "num_tables": bp.sc_num_tables,
            "table_size": bp.sc_table_size,
            "history_lengths": bp.sc_history_lengths,
            "counter_bits": bp.sc_counter_bits,
            "bias_table_size": bp.sc_bias_table_size,
            "bias_counter_bits": bp.sc_bias_counter_bits,
            "initial_threshold": bp.sc_initial_threshold,
            "per_pc_threshold_bits": bp.sc_per_pc_threshold_bits,
        }
    return {
        "num_tables": 6,
        "table_size": 512,
        "history_lengths": [0, 2, 4, 8, 12, 16],
        "counter_bits": 3,
        "bias_table_size": 256,
        "bias_counter_bits": 6,
        "initial_threshold": 35,
        "per_pc_threshold_bits": 6,
    }


def _ittage_sub_dict(bp) -> dict:
    """Return the ITTAGE sub-config dict for ScLTage."""
    if isinstance(bp, BranchPredictor.ScLTage):
        return {
            "num_banks": bp.ittage_num_banks,
            "table_size": bp.ittage_table_size,
            "history_lengths": bp.ittage_history_lengths,
            "tag_widths": bp.ittage_tag_widths,
            "reset_interval": bp.ittage_reset_interval,
        }
    return {
        "num_banks": 8,
        "table_size": 256,
        "history_lengths": [4, 8, 16, 32, 64, 128, 256, 512],
        "tag_widths": [9, 9, 10, 10, 11, 11, 12, 12],
        "reset_interval": 256_000,
    }


def _mdp_name(mdp) -> str:
    """Return the memory dependence predictor name string for the Rust backend."""
    if isinstance(mdp, MemDepPredictor.Blind):
        return "Blind"
    if isinstance(mdp, MemDepPredictor.StoreSet):
        return "StoreSet"
    raise TypeError(f"Unknown memory dependence predictor type: {type(mdp)}")


def _mdp_sub_dict(mdp) -> dict:
    """Return the MDP sub-config dict."""
    if isinstance(mdp, MemDepPredictor.StoreSet):
        return {
            "ssit_size": mdp.ssit_size,
            "lfst_size": mdp.lfst_size,
        }
    return {}


def _replacement_policy_name(policy) -> str:
    """Return the replacement policy name string for the Rust backend."""
    if isinstance(policy, ReplacementPolicy.LRU):
        return "LRU"
    if isinstance(policy, ReplacementPolicy.PLRU):
        return "PLRU"
    if isinstance(policy, ReplacementPolicy.FIFO):
        return "FIFO"
    if isinstance(policy, ReplacementPolicy.Random):
        return "Random"
    if isinstance(policy, ReplacementPolicy.MRU):
        return "MRU"
    raise TypeError(f"Unknown replacement policy type: {type(policy)}")


def _prefetcher_name(pf) -> str:
    """Return the prefetcher name string for the Rust backend."""
    if isinstance(pf, Prefetcher.Off):
        return "None"
    if isinstance(pf, Prefetcher.NextLine):
        return "NextLine"
    if isinstance(pf, Prefetcher.Stride):
        return "Stride"
    if isinstance(pf, Prefetcher.Stream):
        return "Stream"
    if isinstance(pf, Prefetcher.Tagged):
        return "Tagged"
    raise TypeError(f"Unknown prefetcher type: {type(pf)}")


def _prefetcher_degree(pf) -> int:
    """Return the prefetcher degree."""
    if isinstance(
        pf,
        (Prefetcher.NextLine, Prefetcher.Stride, Prefetcher.Stream, Prefetcher.Tagged),
    ):
        return pf.degree
    return 0


def _prefetcher_table_size(pf) -> int:
    """Return the prefetcher table size."""
    if isinstance(pf, Prefetcher.Stride):
        return pf.table_size
    return 0


def _inclusion_policy_name(ip) -> str:
    """Return the inclusion policy name string for the Rust backend."""
    if isinstance(ip, Cache.NINE):
        return "NINE"
    if isinstance(ip, Cache.Inclusive):
        return "Inclusive"
    if isinstance(ip, Cache.Exclusive):
        return "Exclusive"
    raise TypeError(f"Unknown inclusion policy type: {type(ip)}")


def _mc_name(mc) -> str:
    """Return the memory controller name string for the Rust backend."""
    if isinstance(mc, MemoryController.Simple):
        return "Simple"
    if isinstance(mc, MemoryController.DRAM):
        return "Dram"
    raise TypeError(f"Unknown memory controller type: {type(mc)}")


def _backend_name(be) -> str:
    """Return the backend name string for the Rust backend."""
    if isinstance(be, Backend.InOrder):
        return "InOrder"
    if isinstance(be, Backend.OutOfOrder):
        return "OutOfOrder"
    raise TypeError(f"Unknown backend type: {type(be)}")


def _fu_config_to_dict(fc: Fu) -> dict:
    """Serialize a Fu pool config to the flat dict the Rust backend expects."""
    # Start with zeroed-out defaults for every FU type so the Rust serde
    # always finds every key, even if the user omits a unit type entirely.
    d = {
        "num_int_alu": 0,
        "int_alu_latency": 1,
        "num_int_mul": 0,
        "int_mul_latency": 3,
        "num_int_div": 0,
        "int_div_latency": 35,
        "num_fp_add": 0,
        "fp_add_latency": 4,
        "num_fp_mul": 0,
        "fp_mul_latency": 5,
        "num_fp_fma": 0,
        "fp_fma_latency": 5,
        "num_fp_div_sqrt": 0,
        "fp_div_sqrt_latency": 21,
        "num_branch": 0,
        "branch_latency": 1,
        "num_mem": 0,
        "mem_latency": 1,
    }
    for u in fc.units:
        if isinstance(u, Fu.IntAlu):
            d["num_int_alu"] = u.count
            d["int_alu_latency"] = u.latency
        elif isinstance(u, Fu.IntMul):
            d["num_int_mul"] = u.count
            d["int_mul_latency"] = u.latency
        elif isinstance(u, Fu.IntDiv):
            d["num_int_div"] = u.count
            d["int_div_latency"] = u.latency
        elif isinstance(u, Fu.FpAdd):
            d["num_fp_add"] = u.count
            d["fp_add_latency"] = u.latency
        elif isinstance(u, Fu.FpMul):
            d["num_fp_mul"] = u.count
            d["fp_mul_latency"] = u.latency
        elif isinstance(u, Fu.FpFma):
            d["num_fp_fma"] = u.count
            d["fp_fma_latency"] = u.latency
        elif isinstance(u, Fu.FpDivSqrt):
            d["num_fp_div_sqrt"] = u.count
            d["fp_div_sqrt_latency"] = u.latency
        elif isinstance(u, Fu.Branch):
            d["num_branch"] = u.count
            d["branch_latency"] = u.latency
        elif isinstance(u, Fu.Mem):
            d["num_mem"] = u.count
            d["mem_latency"] = u.latency
        else:
            raise TypeError(f"Unknown Fu type: {type(u)}")
    return d


def _backend_to_pipeline_fields(be) -> dict:
    """Return pipeline-level fields that come from the backend object."""
    if isinstance(be, Backend.OutOfOrder):
        return {
            "rob_size": be.rob_size,
            "store_buffer_size": be.store_buffer_size,
            "issue_queue_size": be.issue_queue_size,
            "load_queue_size": be.load_queue_size,
            "load_ports": be.load_ports,
            "store_ports": be.store_ports,
            "prf_gpr_size": be.prf_gpr_size,
            "prf_fpr_size": be.prf_fpr_size,
            "fu_config": _fu_config_to_dict(be.fu_config),
            "checkpoint_count": be.checkpoint_count,
        }
    # InOrder: emit safe defaults so Rust serde never chokes on missing keys
    return {
        "rob_size": 64,
        "store_buffer_size": 16,
        "issue_queue_size": 32,
        "load_queue_size": 32,
        "load_ports": 1,
        "store_ports": 1,
        "prf_gpr_size": 64,
        "prf_fpr_size": 64,
        "fu_config": _fu_config_to_dict(Fu()),
    }


def _cache_to_dict(c: Cache) -> Dict[str, Any]:
    """Serialize a Cache object to the dict format the Rust backend expects."""
    d: Dict[str, Any] = {
        "enabled": True,
        "size_bytes": c.size_bytes,
        "line_bytes": c.line_bytes,
        "ways": c.ways,
        "policy": _replacement_policy_name(c.policy),
        "latency": c.latency,
        "prefetcher": _prefetcher_name(c.prefetcher),
        "prefetch_table_size": _prefetcher_table_size(c.prefetcher),
        "prefetch_degree": _prefetcher_degree(c.prefetcher),
    }
    if c.mshr_count > 0:
        d["mshr_count"] = c.mshr_count
    return d


_TAGE_DEFAULTS = {
    "num_banks": 8,
    "table_size": 2048,
    "loop_table_size": 256,
    "reset_interval": 256_000,
    "history_lengths": [5, 11, 22, 44, 89, 178, 356, 712],
    "tag_widths": [8, 8, 9, 9, 10, 10, 11, 11],
}

_PERCEPTRON_DEFAULTS = {
    "history_length": 32,
    "table_bits": 10,
}

_TOURNAMENT_DEFAULTS = {
    "global_size_bits": 12,
    "local_hist_bits": 10,
    "local_pred_bits": 10,
}


def _config_to_dict_impl(cfg: Config) -> Dict[str, Any]:
    """Produce the nested dict expected by the Rust backend."""
    # General
    general: Dict[str, Any] = {
        "trace_instructions": cfg.trace,
        "start_pc": _START_PC_DEFAULT,
        "direct_mode": True,
    }
    if cfg.initial_sp is not None:
        general["initial_sp"] = cfg.initial_sp

    # System
    system = {
        "ram_base": cfg.ram_base,
        "uart_base": cfg.uart_base,
        "disk_base": cfg.disk_base,
        "clint_base": cfg.clint_base,
        "syscon_base": cfg.syscon_base,
        "kernel_offset": cfg.kernel_offset,
        "bus_width": cfg.bus_width,
        "bus_latency": cfg.bus_latency,
        "clint_divider": cfg.clint_divider,
        "uart_to_stderr": cfg.uart_to_stderr,
        "uart_quiet": cfg.uart_quiet,
        "tohost_addr": 0,
    }

    # Memory — merge controller-specific params
    mc = cfg.memory_controller
    memory: Dict[str, Any] = {
        "ram_size": cfg.ram_size,
        "controller": _mc_name(mc),
        "tlb_size": cfg.tlb_size,
        "l2_tlb_size": cfg.l2_tlb_size,
        "l2_tlb_ways": cfg.l2_tlb_ways,
        "l2_tlb_latency": cfg.l2_tlb_latency,
        "software_ad_bits": cfg.software_ad_bits,
        "misaligned_access_trap": cfg.misaligned_access_trap,
    }
    # Always emit DRAM timing keys (Rust expects them)
    if isinstance(mc, MemoryController.DRAM):
        memory["t_cas"] = mc.t_cas
        memory["t_ras"] = mc.t_ras
        memory["t_pre"] = mc.t_pre
        memory["row_miss_latency"] = mc.row_miss_latency
    else:
        memory["t_cas"] = 14
        memory["t_ras"] = 14
        memory["t_pre"] = 14
        memory["row_miss_latency"] = 120

    # Caches
    cache = {
        "l1_i": (
            _cache_to_dict(cfg.l1i) if cfg.l1i is not None else _DISABLED_CACHE_DICT
        ),
        "l1_d": (
            _cache_to_dict(cfg.l1d) if cfg.l1d is not None else _DISABLED_CACHE_DICT
        ),
        "l2": (
            _cache_to_dict(cfg.l2) if cfg.l2 is not None else _DISABLED_CACHE_DICT_ZERO
        ),
        "l3": (
            _cache_to_dict(cfg.l3) if cfg.l3 is not None else _DISABLED_CACHE_DICT_ZERO
        ),
        "inclusion_policy": _inclusion_policy_name(cfg.inclusion_policy),
        "wcb_entries": cfg.wcb_entries,
    }

    # Pipeline — always emit all BP sub-configs with defaults
    bp = cfg.branch_predictor
    tage_dict = (
        _bp_sub_dict(bp)
        if isinstance(bp, (BranchPredictor.TAGE, BranchPredictor.ScLTage))
        else _TAGE_DEFAULTS
    )
    perceptron_dict = (
        _bp_sub_dict(bp)
        if isinstance(bp, BranchPredictor.Perceptron)
        else _PERCEPTRON_DEFAULTS
    )
    tournament_dict = (
        _bp_sub_dict(bp)
        if isinstance(bp, BranchPredictor.Tournament)
        else _TOURNAMENT_DEFAULTS
    )
    sc_dict = _sc_sub_dict(bp)
    ittage_dict = _ittage_sub_dict(bp)

    # MDP sub-config
    mdp = cfg.mem_dep_predictor
    store_set_dict = (
        _mdp_sub_dict(mdp)
        if isinstance(mdp, MemDepPredictor.StoreSet)
        else {"ssit_size": 2048, "lfst_size": 256}
    )

    pipeline = {
        "width": cfg.width,
        "branch_predictor": _bp_name(bp),
        "btb_size": cfg.btb_size,
        "btb_ways": cfg.btb_ways,
        "ras_size": cfg.ras_size,
        "backend": _backend_name(cfg.backend),
        "tage": tage_dict,
        "perceptron": perceptron_dict,
        "tournament": tournament_dict,
        "sc": sc_dict,
        "ittage": ittage_dict,
        "mem_dep_predictor": _mdp_name(mdp),
        "store_set": store_set_dict,
        **_backend_to_pipeline_fields(cfg.backend),
    }

    return {
        "general": general,
        "system": system,
        "memory": memory,
        "cache": cache,
        "pipeline": pipeline,
    }
