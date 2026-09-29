"""Turning a ``Config`` into the nested dict the Rust core deserializes."""

from __future__ import annotations


from typing import TYPE_CHECKING, Any, Dict

from .backend import Backend, Fu
from .branch import BranchPredictor, MemDepPredictor
from .coherence import Coherence, HomeAgent, Interconnect
from .memory import (
    _DISABLED_CACHE_DICT,
    _DISABLED_CACHE_DICT_ZERO,
    Cache,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
)

if TYPE_CHECKING:
    from ._config import Config


_START_PC_DEFAULT = 0x8000_0000


def _console_name(cfg) -> str:
    """Where the UART console connects: ``console`` if given, else what
    the ``uart_quiet`` / ``uart_to_stderr`` shorthands say."""
    if cfg.console is not None:
        return cfg.console
    if cfg.uart_quiet:
        return "quiet"
    if cfg.uart_to_stderr:
        return "stderr"
    return "stdout"


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
            "reset_interval": bp.reset_interval,
            "history_lengths": bp.history_lengths,
            "tag_widths": bp.tag_widths,
            "use_alt_counters": bp.use_alt_counters,
            "use_alt_bits": bp.use_alt_bits,
            "useful_bits": bp.useful_bits,
            "max_allocations": bp.max_allocations,
            "allocation": bp.allocation,
            "update": bp.update,
            "history": bp.history,
            "path_history_bits": bp.path_history_bits,
            "bimodal_entries": bp.bimodal_entries,
            "bimodal_hysteresis_share_log": bp.bimodal_hysteresis_share_log,
            "hashing": bp.hashing,
            "banking": bp.banking.to_dict() if bp.banking is not None else None,
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
    if not isinstance(bp, BranchPredictor.ScLTage):
        bp = BranchPredictor.ScLTage()
    return {
        "log_bias": bp.sc_log_bias,
        "counter_bits": bp.sc_counter_bits,
        "weight_bits": bp.sc_weight_bits,
        "bias_weight_init": bp.sc_bias_weight_init,
        "chooser_bits": bp.sc_chooser_bits,
        "threshold_bits": bp.sc_threshold_bits,
        "initial_threshold": bp.sc_initial_threshold,
        "per_pc_threshold_bits": bp.sc_per_pc_threshold_bits,
        "per_pc_threshold_width": bp.sc_per_pc_threshold_width,
        "initial_per_pc_threshold": bp.sc_initial_per_pc_threshold,
        "threshold_weight_step": bp.sc_threshold_weight_step,
        "halve_short_tables": bp.sc_halve_short_tables,
        "imli_counter_bits": bp.sc_imli_counter_bits,
        "global": bp.sc_global.to_dict(),
        "backward": bp.sc_backward.to_dict(),
        "path": bp.sc_path.to_dict(),
        "local": [local.to_dict() for local in bp.sc_local],
        "imli": bp.sc_imli.to_dict(),
        "imli_history": bp.sc_imli_history.to_dict(),
    }


_LOOP_FIELDS = (
    "log_size",
    "log_assoc",
    "tag_bits",
    "iter_bits",
    "confidence_bits",
    "age_bits",
    "use_counter_bits",
    "use_direction_bit",
    "use_hashing",
    "restrict_allocation",
    "initial_iter",
    "initial_age",
    "optional_age_reset",
    "long_loop_confidence",
    "optional_age_increment",
)


def _loop_sub_dict(bp) -> dict:
    """Return the loop predictor sub-config dict for ScLTage."""
    if not isinstance(bp, BranchPredictor.ScLTage):
        bp = BranchPredictor.ScLTage()
    return {field: getattr(bp, f"loop_{field}") for field in _LOOP_FIELDS}


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
    if isinstance(mc, MemoryController.DDR5):
        return "Ddr5"
    raise TypeError(f"Unknown memory controller type: {type(mc)}")


def _coherence_to_dict(c: Coherence) -> Dict[str, Any]:
    """Serialize a Coherence object to the dict format the Rust backend expects."""
    home = c.home_agent
    if isinstance(home, HomeAgent.Broadcast):
        home_dict: Dict[str, Any] = {"kind": "Broadcast"}
    elif isinstance(home, HomeAgent.SnoopFilter):
        home_dict = {"kind": "SnoopFilter", "capacity_factor": home.capacity_factor, "ways": home.ways}
    else:
        raise TypeError(f"Unknown home agent type: {type(home)}")
    ic = c.interconnect
    if not isinstance(ic, Interconnect._Kind):
        raise TypeError(f"Unknown interconnect type: {type(ic)}")
    return {
        "protocol": "Mesi",
        "home_agent": home_dict,
        "interconnect": {"kind": ic.kind, "hop_latency": ic.hop_latency, "bytes_per_cycle": ic.bytes_per_cycle},
        "txn_entries": c.txn_entries,
    }


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
        "num_vec_int_alu": 0,
        "vec_int_alu_latency": 1,
        "num_vec_int_mul": 0,
        "vec_int_mul_latency": 3,
        "num_vec_int_div": 0,
        "vec_int_div_latency": 20,
        "num_vec_fp_alu": 0,
        "vec_fp_alu_latency": 4,
        "num_vec_fp_fma": 0,
        "vec_fp_fma_latency": 5,
        "num_vec_fp_div_sqrt": 0,
        "vec_fp_div_sqrt_latency": 20,
        "num_vec_mem": 0,
        "vec_mem_latency": 1,
        "num_vec_permute": 0,
        "vec_permute_latency": 1,
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
        elif isinstance(u, Fu.VecIntAlu):
            d["num_vec_int_alu"] = u.count
            d["vec_int_alu_latency"] = u.latency
        elif isinstance(u, Fu.VecIntMul):
            d["num_vec_int_mul"] = u.count
            d["vec_int_mul_latency"] = u.latency
        elif isinstance(u, Fu.VecIntDiv):
            d["num_vec_int_div"] = u.count
            d["vec_int_div_latency"] = u.latency
        elif isinstance(u, Fu.VecFpAlu):
            d["num_vec_fp_alu"] = u.count
            d["vec_fp_alu_latency"] = u.latency
        elif isinstance(u, Fu.VecFpFma):
            d["num_vec_fp_fma"] = u.count
            d["vec_fp_fma_latency"] = u.latency
        elif isinstance(u, Fu.VecFpDivSqrt):
            d["num_vec_fp_div_sqrt"] = u.count
            d["vec_fp_div_sqrt_latency"] = u.latency
        elif isinstance(u, Fu.VecMem):
            d["num_vec_mem"] = u.count
            d["vec_mem_latency"] = u.latency
        elif isinstance(u, Fu.VecPermute):
            d["num_vec_permute"] = u.count
            d["vec_permute_latency"] = u.latency
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
            "prf_vpr_size": be.prf_vpr_size,
            "vec_chaining": be.vec_chaining,
            "vec_store_buffer_size": be.vec_store_buffer_size,
            "vec_store_forwarding": be.vec_store_forwarding,
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
        "prf_vpr_size": 64,
        "vec_chaining": True,
        "vec_store_buffer_size": 8,
        "vec_store_forwarding": "byte_mask",
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
        "response_latency": c.response_latency,
        "prefetcher": _prefetcher_name(c.prefetcher),
        "prefetch_table_size": _prefetcher_table_size(c.prefetcher),
        "prefetch_degree": _prefetcher_degree(c.prefetcher),
    }
    if c.mshr_count > 0:
        d["mshr_count"] = c.mshr_count
    if c.write_buffers > 0:
        d["write_buffers"] = c.write_buffers
    if c.targets_per_mshr > 0:
        d["targets_per_mshr"] = c.targets_per_mshr
    return d


_TAGE_DEFAULTS = {
    "num_banks": 8,
    "table_size": 2048,
    "reset_interval": 256_000,
    "history_lengths": [5, 11, 22, 44, 89, 178, 356, 712],
    "tag_widths": [8, 8, 9, 9, 10, 10, 11, 11],
    "use_alt_counters": 1,
    "use_alt_bits": 4,
    "useful_bits": 2,
    "max_allocations": 1,
    "allocation": "tage_base",
    "update": "tage_base",
    "history": "direction",
    "path_history_bits": 16,
    "bimodal_entries": None,
    "bimodal_hysteresis_share_log": 2,
    "hashing": "tage_base",
    "banking": None,
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
        "sim_control_base": cfg.sim_control_base,
        "kernel_offset": cfg.kernel_offset,
        "bus_width": cfg.bus_width,
        "bus_latency": cfg.bus_latency,
        "clint_divider": cfg.clint_divider,
        "cpu_clock_mhz": cfg.cpu_clock_mhz,
        "device_latency_ns": cfg.device_latency_ns,
        "device_latency_ns_overrides": cfg.device_latency_ns_overrides,
        "rtc_epoch_seconds": cfg.rtc_epoch_seconds,
        "console": _console_name(cfg),
        "tohost_addr": 0,
        "hart_count": cfg.hart_count,
    }

    # Memory — merge controller-specific params
    mc = cfg.memory_controller
    memory: Dict[str, Any] = {
        "ram_size": cfg.ram_size,
        "controller": _mc_name(mc),
        "tlb_size": cfg.tlb_size,
        "tlb_ways": cfg.tlb_ways,
        "l2_tlb_size": cfg.l2_tlb_size,
        "l2_tlb_ways": cfg.l2_tlb_ways,
        "l2_tlb_latency": cfg.l2_tlb_latency,
        "paging_mode_max": cfg.paging_mode_max,
        "misaligned_access_trap": cfg.misaligned_access_trap,
    }
    if isinstance(mc, MemoryController.Simple):
        memory["simple_bandwidth_gib_s"] = mc.bandwidth_gib_s
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
    if isinstance(mc, MemoryController.DDR5):
        memory["ddr5"] = mc.to_dict()

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
    loop_dict = _loop_sub_dict(bp)

    # MDP sub-config
    mdp = cfg.mem_dep_predictor
    store_set_dict = (
        _mdp_sub_dict(mdp)
        if isinstance(mdp, MemDepPredictor.StoreSet)
        else {"ssit_size": 2048, "lfst_size": 256}
    )

    pipeline = {
        "width": cfg.width,
        "fetch_width": cfg.fetch_width,
        "decode_width": cfg.decode_width,
        "rename_width": cfg.rename_width,
        "issue_width": cfg.issue_width,
        "commit_width": cfg.commit_width,
        "writeback_width": cfg.writeback_width,
        "trap_latency": cfg.trap_latency,
        "redirect_latency": cfg.redirect_latency,
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
        "loop_predictor": loop_dict,
        "mem_dep_predictor": _mdp_name(mdp),
        "store_set": store_set_dict,
        "vlen": cfg.vlen,
        **_backend_to_pipeline_fields(cfg.backend),
    }
    if cfg.num_vec_lanes is not None:
        pipeline["num_vec_lanes"] = cfg.num_vec_lanes
    if cfg.vector_mem_width is not None:
        pipeline["vector_mem_width"] = cfg.vector_mem_width

    return {
        "general": general,
        "isa": {"svadu": cfg.svadu},
        "system": system,
        "memory": memory,
        "cache": cache,
        "coherence": _coherence_to_dict(cfg.coherence),
        "pipeline": pipeline,
    }
