"""Turning a ``Config`` into the nested dict the Rust core deserializes. It
writes what the Python objects model, under the core's names, and leaves
every other field to the core's own defaults."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from .backend import _UNITS, Backend, Fu
from .branch import BranchPredictor, MemDepPredictor
from .coherence import Coherence, HomeAgent, Interconnect
from .memory import (
    Cache,
    LoadPrefetcher,
    MemoryController,
    PageBoundary,
    Prefetcher,
    ReplacementPolicy,
    StorePrefetcher,
)

if TYPE_CHECKING:
    from ._config import Config


def _name(namespace: type, value: object) -> str:
    """The core's name for ``value``'s kind: its class's name, which must be
    one of ``namespace``'s kinds."""
    name = type(value).__name__
    if getattr(namespace, name, None) is not type(value):
        raise TypeError(f"Unknown {namespace.__name__} kind: {value!r}")
    return name


def _console_name(cfg) -> str:
    """Where the UART console connects: ``console`` if given, else what
    the ``uart_quiet`` / ``uart_to_stderr`` shorthands say."""
    if cfg.console is not None:
        return cfg.console
    if cfg.uart_quiet:
        return "Quiet"
    if cfg.uart_to_stderr:
        return "Stderr"
    return "Stdout"


def _tage_dict(bp: BranchPredictor.Tage | BranchPredictor.ScLTage) -> dict[str, Any]:
    """A TAGE or TAGE-SC-L predictor's TAGE tables."""
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


_SC_SCALARS = (
    "log_bias",
    "counter_bits",
    "weight_bits",
    "bias_weight_init",
    "chooser_bits",
    "threshold_bits",
    "initial_threshold",
    "per_pc_threshold_bits",
    "per_pc_threshold_width",
    "initial_per_pc_threshold",
    "threshold_weight_step",
    "halve_short_tables",
    "imli_counter_bits",
)


def _tage_sc_l_dicts(bp: BranchPredictor.ScLTage) -> dict[str, Any]:
    """A TAGE-SC-L predictor's TAGE, statistical corrector, ITTAGE and loop
    predictor."""
    sc = {field: getattr(bp, f"sc_{field}") for field in _SC_SCALARS}
    sc.update(
        {
            "global": bp.sc_global.to_dict(),
            "backward": bp.sc_backward.to_dict(),
            "path": bp.sc_path.to_dict(),
            "local": [local.to_dict() for local in bp.sc_local],
            "imli": bp.sc_imli.to_dict(),
            "imli_history": bp.sc_imli_history.to_dict(),
        }
    )
    loop_fields = (
        name.removeprefix("loop_") for name in vars(bp) if name.startswith("loop_")
    )
    return {
        "tage": _tage_dict(bp),
        "sc": sc,
        "ittage": {
            "num_banks": bp.ittage_num_banks,
            "table_size": bp.ittage_table_size,
            "history_lengths": bp.ittage_history_lengths,
            "tag_widths": bp.ittage_tag_widths,
            "reset_interval": bp.ittage_reset_interval,
        },
        "loop_predictor": {
            field: getattr(bp, f"loop_{field}") for field in loop_fields
        },
    }


def _predictor_dicts(bp) -> dict[str, Any]:
    """The pipeline fields the selected branch predictor sets: its name and
    its own tables."""
    fields: dict[str, Any] = {"branch_predictor": _name(BranchPredictor, bp)}
    if isinstance(bp, BranchPredictor.ScLTage):
        fields.update(_tage_sc_l_dicts(bp))
    elif isinstance(bp, BranchPredictor.Tage):
        fields["tage"] = _tage_dict(bp)
    elif isinstance(bp, BranchPredictor.Perceptron):
        fields["perceptron"] = {
            "history_length": bp.history_length,
            "table_bits": bp.table_bits,
        }
    elif isinstance(bp, BranchPredictor.Tournament):
        fields["tournament"] = {
            "global_size_bits": bp.global_size_bits,
            "local_hist_bits": bp.local_hist_bits,
            "local_pred_bits": bp.local_pred_bits,
        }
    elif isinstance(bp, BranchPredictor.Bht):
        fields["bht"] = {
            "entries": bp.entries,
            "counter_bits": bp.counter_bits,
            "history_length": bp.history_length,
            "history_bits": bp.history_bits,
        }
    return fields


def _mem_dep_dicts(mdp) -> dict[str, Any]:
    """The pipeline fields the memory dependence predictor sets."""
    fields: dict[str, Any] = {"mem_dep_predictor": _name(MemDepPredictor, mdp)}
    if isinstance(mdp, MemDepPredictor.StoreSet):
        fields["store_set"] = {"ssit_size": mdp.ssit_size, "lfst_size": mdp.lfst_size}
    return fields


def _fu_config_to_dict(fc: Fu) -> dict[str, int]:
    """A unit list as the core's pool: a unit type the list leaves out has
    no units, and keeps the core's latency."""
    stems = {unit: stem for unit, stem in _UNITS}
    pool = {f"num_{stem}": 0 for _, stem in _UNITS}
    for unit in fc.units:
        stem = stems.get(type(unit))
        if stem is None:
            raise TypeError(f"Unknown Fu type: {type(unit)}")
        pool[f"num_{stem}"] = unit.count
        pool[f"{stem}_latency"] = unit.latency
    return pool


def _backend_dicts(be) -> dict[str, Any]:
    """The pipeline fields the backend sets: both backends' ROB, store
    buffer, ports and vector store buffer, and the out-of-order backend's
    queues and register files."""
    fields: dict[str, Any] = {
        "backend": _name(Backend, be),
        "fu_config": _fu_config_to_dict(be.fu_config),
        "rob_size": be.rob_size,
        "store_buffer_size": be.store_buffer_size,
        "load_ports": be.load_ports,
        "store_ports": be.store_ports,
        "vec_store_buffer_size": be.vec_store_buffer_size,
        "vec_store_forwarding": be.vec_store_forwarding,
    }
    if isinstance(be, Backend.OutOfOrder):
        fields.update(
            {
                "issue_queue_size": be.issue_queue_size,
                "load_queue_size": be.load_queue_size,
                "prf_gpr_size": be.prf_gpr_size,
                "prf_fpr_size": be.prf_fpr_size,
                "checkpoint_count": be.checkpoint_count,
                "squash_width": be.squash_width,
                "prf_vpr_size": be.prf_vpr_size,
                "vec_chaining": be.vec_chaining,
            }
        )
    return fields


def _cache_to_dict(c: Cache | None) -> dict[str, Any]:
    """A cache level, or an absent one: disabled, at the core's geometry."""
    if c is None:
        return {"enabled": False}
    pf = c.prefetcher
    d: dict[str, Any] = {
        "enabled": True,
        "size_bytes": c.size_bytes,
        "line_bytes": c.line_bytes,
        "ways": c.ways,
        "policy": _name(ReplacementPolicy, c.policy),
        "latency": c.latency,
        "response_latency": c.response_latency,
        "prefetcher": _name(Prefetcher, pf),
    }
    if not isinstance(pf, Prefetcher.Off):
        d["prefetch_degree"] = pf.degree
    if isinstance(pf, Prefetcher.Stride):
        d["prefetch_table_size"] = pf.table_size
    for field in ("mshr_count", "write_buffers", "targets_per_mshr"):
        if getattr(c, field) is not None:
            d[field] = getattr(c, field)
    return d


def _load_prefetcher_to_dict(p: LoadPrefetcher.Stride | None) -> dict[str, Any]:
    """The load/store unit's load prefetcher."""
    if p is None:
        return {"kind": "None"}
    if not isinstance(p, LoadPrefetcher.Stride):
        raise TypeError(f"Unknown load prefetcher type: {type(p)}")
    return {
        "kind": "Stride",
        "table_size": p.table_size,
        "l1_lines": p.l1_lines,
        "l2_lines": p.l2_lines,
        "page_boundary": _name(PageBoundary, p.page_boundary),
    }


def _store_prefetcher_to_dict(p: StorePrefetcher.Stream | None) -> dict[str, Any]:
    """The L1D's store-miss prefetcher."""
    if p is None:
        return {"kind": "None"}
    if not isinstance(p, StorePrefetcher.Stream):
        raise TypeError(f"Unknown store prefetcher type: {type(p)}")
    return {"kind": "Stream", "streams": p.streams, "l2_lines": p.l2_lines}


def _controller_dicts(mc) -> dict[str, Any]:
    """The memory fields the memory controller sets."""
    fields: dict[str, Any] = {"controller": _name(MemoryController, mc)}
    if isinstance(mc, MemoryController.Simple):
        fields["simple_latency"] = mc.latency
        fields["simple_bandwidth_gib_s"] = mc.bandwidth_gib_s
    elif isinstance(mc, MemoryController.Dram):
        fields.update({"t_cas": mc.t_cas, "t_ras": mc.t_ras, "t_pre": mc.t_pre})
    elif isinstance(mc, MemoryController.Ddr5):
        fields["ddr5"] = mc.to_dict()
    return fields


def _coherence_to_dict(c: Coherence) -> dict[str, Any]:
    """The coherence fabric between the private caches."""
    home = c.home_agent
    home_dict: dict[str, Any] = {"kind": _name(HomeAgent, home)}
    if isinstance(home, HomeAgent.SnoopFilter):
        home_dict.update({"capacity_factor": home.capacity_factor, "ways": home.ways})
    ic = c.interconnect
    if not isinstance(ic, Interconnect._Kind):
        raise TypeError(f"Unknown interconnect type: {type(ic)}")
    return {
        "protocol": "Mesi",
        "home_agent": home_dict,
        "interconnect": {
            "kind": ic.kind,
            "hop_latency": ic.hop_latency,
            "bytes_per_cycle": ic.bytes_per_cycle,
        },
        "txn_entries": c.txn_entries,
    }


def _config_to_dict_impl(cfg: Config) -> dict[str, Any]:
    """Produce the nested dict expected by the Rust backend."""
    general = {
        "trace_instructions": cfg.trace,
        "page_mode": cfg.page_mode,
        "clock_hz": cfg.clock_hz,
    }
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
        "cpu_clock_mhz": cfg.cpu_clock_mhz,
        "device_latency_ns": cfg.device_latency_ns,
        "device_latency_ns_overrides": cfg.device_latency_ns_overrides,
        "rtc_epoch_seconds": cfg.rtc_epoch_seconds,
        "console": _console_name(cfg),
        "hart_count": cfg.hart_count,
    }
    memory = {
        "ram_size": cfg.ram_size,
        "tlb_size": cfg.tlb_size,
        "tlb_ways": cfg.tlb_ways,
        "l2_tlb_size": cfg.l2_tlb_size,
        "l2_tlb_ways": cfg.l2_tlb_ways,
        "l2_tlb_latency": cfg.l2_tlb_latency,
        "paging_mode_max": cfg.paging_mode_max,
        "misaligned_access_trap": cfg.misaligned_access_trap,
        **_controller_dicts(cfg.memory_controller),
    }
    cache = {
        "l1_i": _cache_to_dict(cfg.l1i),
        "l1_d": _cache_to_dict(cfg.l1d),
        "l2": _cache_to_dict(cfg.l2),
        "l3": _cache_to_dict(cfg.l3),
        "inclusion_policy": _name(Cache, cfg.inclusion_policy),
        "wcb_entries": cfg.wcb_entries,
        "load_prefetcher": _load_prefetcher_to_dict(cfg.load_prefetcher),
        "store_prefetcher": _store_prefetcher_to_dict(cfg.store_prefetcher),
    }
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
        "fetch_decode_latency": cfg.fetch_decode_latency,
        "decode_rename_latency": cfg.decode_rename_latency,
        "rename_issue_latency": cfg.rename_issue_latency,
        "csr_squash": cfg.csr_squash,
        "fence_squash": cfg.fence_squash,
        "store_forward_latency": cfg.store_forward_latency,
        "btb_size": cfg.btb_size,
        "btb_ways": cfg.btb_ways,
        "ras_size": cfg.ras_size,
        "vlen": cfg.vlen,
        "num_vec_lanes": cfg.num_vec_lanes,
        "vector_mem_width": cfg.vector_mem_width,
        **_predictor_dicts(cfg.branch_predictor),
        **_mem_dep_dicts(cfg.mem_dep_predictor),
        **_backend_dicts(cfg.backend),
    }
    isa: dict[str, Any] = {"svadu": cfg.svadu}
    if cfg.isa is not None:
        isa["string"] = cfg.isa
    return {
        "general": general,
        "isa": isa,
        "system": system,
        "memory": memory,
        "cache": cache,
        "coherence": _coherence_to_dict(cfg.coherence),
        "pipeline": pipeline,
    }
