"""
Namespace types for simulator configuration.

Provides structured, Pythonic alternatives to raw string enums:
- BranchPredictor: Static, GShare, TAGE, Perceptron, Tournament
- MemDepPredictor: Blind, StoreSet
- ReplacementPolicy: LRU, PLRU, FIFO, Random, MRU
- Prefetcher: Off, NextLine, Stride, Stream, Tagged
- MemoryController: Simple, DRAM
- Backend: InOrder, OutOfOrder
- Cache: cache level configuration with size parsing
"""

from __future__ import annotations

import re
from typing import Any, Dict, List, Optional

__all__ = [
    "BranchPredictor",
    "ReplacementPolicy",
    "Prefetcher",
    "MemoryController",
    "Backend",
    "Cache",
    "Fu",
]


def _parse_size(s) -> int:
    """Parse a size string like '32KB', '4MB', '64B' into bytes. Ints pass through."""
    if isinstance(s, int):
        return s
    if not isinstance(s, str):
        raise TypeError(f"Expected str or int, got {type(s).__name__}")
    m = re.fullmatch(r"(\d+)\s*(B|KB|MB|GB)", s.strip(), re.IGNORECASE)
    if not m:
        raise ValueError(f"Cannot parse size: {s!r} (expected e.g. '32KB', '4MB')")
    val = int(m.group(1))
    unit = m.group(2).upper()
    mult = {"B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3}
    return val * mult[unit]


def _parse_cycles(s) -> int:
    """Parse a cycle count like ``'5M'``, ``'500K'``, ``'2G'`` into an integer.

    Uses SI (decimal) suffixes: K = 1,000, M = 1,000,000, G = 1,000,000,000.
    Plain integers and numeric strings pass through.
    """
    if isinstance(s, int):
        return s
    if not isinstance(s, str):
        raise TypeError(f"Expected str or int, got {type(s).__name__}")
    m = re.fullmatch(r"(\d+)\s*([KMG])?", s.strip(), re.IGNORECASE)
    if not m:
        raise ValueError(
            f"Cannot parse cycle count: {s!r} (expected e.g. '5M', '500K', or plain integer)"
        )
    val = int(m.group(1))
    suffix = (m.group(2) or "").upper()
    mult = {"": 1, "K": 1_000, "M": 1_000_000, "G": 1_000_000_000}
    return val * mult[suffix]


# ── Branch Predictor ─────────────────────────────────────────────────────────

_GHR_MAX_BITS = 1024  # Must match GHR_MAX_WORDS * 64 in branch_predictor.rs


def _validate_history_lengths(lengths: List[int], name: str) -> None:
    """Validate that no history length exceeds the GHR capacity."""
    max_len = max(lengths) if lengths else 0
    if max_len > _GHR_MAX_BITS:
        raise ValueError(
            f"{name}: maximum history length {max_len} exceeds the "
            f"GHR capacity of {_GHR_MAX_BITS} bits. "
            f"All history lengths must be <= {_GHR_MAX_BITS}."
        )


class BranchPredictor:
    """Namespace for branch predictor configurations."""

    class Static:
        def __repr__(self) -> str:
            return "BranchPredictor.Static()"

    class GShare:
        def __repr__(self) -> str:
            return "BranchPredictor.GShare()"

    class TAGE:
        def __init__(
            self,
            num_banks: int = 8,
            table_size: int = 2048,
            loop_table_size: int = 256,
            reset_interval: int = 256_000,
            history_lengths: Optional[List[int]] = None,
            tag_widths: Optional[List[int]] = None,
        ):
            self.num_banks = num_banks
            self.table_size = table_size
            self.loop_table_size = loop_table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths
                if history_lengths is not None
                else [5, 11, 22, 44, 89, 178, 356, 712]
            )
            self.tag_widths = (
                tag_widths
                if tag_widths is not None
                else [8, 8, 9, 9, 10, 10, 11, 11]
            )
            _validate_history_lengths(
                self.history_lengths, "TAGE history_lengths"
            )

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.TAGE(num_banks={self.num_banks}, "
                f"table_size={self.table_size}, "
                f"loop_table_size={self.loop_table_size}, "
                f"reset_interval={self.reset_interval}, "
                f"history_lengths={self.history_lengths}, "
                f"tag_widths={self.tag_widths})"
            )

    class Perceptron:
        def __init__(self, history_length: int = 32, table_bits: int = 10):
            self.history_length = history_length
            self.table_bits = table_bits

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.Perceptron(history_length={self.history_length}, "
                f"table_bits={self.table_bits})"
            )

    class Tournament:
        def __init__(
            self,
            global_size_bits: int = 12,
            local_hist_bits: int = 10,
            local_pred_bits: int = 10,
        ):
            self.global_size_bits = global_size_bits
            self.local_hist_bits = local_hist_bits
            self.local_pred_bits = local_pred_bits

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.Tournament(global_size_bits={self.global_size_bits}, "
                f"local_hist_bits={self.local_hist_bits}, "
                f"local_pred_bits={self.local_pred_bits})"
            )

    class ScLTage:
        """SC-L-TAGE + ITTAGE composed predictor.

        Combines TAGE (direction), Loop Predictor, Statistical Corrector,
        and Indirect Target TAGE into a single high-accuracy predictor.

        The TAGE parameters are shared with the standalone TAGE config.
        SC and ITTAGE have their own sub-configs.
        """

        def __init__(
            self,
            # TAGE parameters
            num_banks: int = 8,
            table_size: int = 2048,
            loop_table_size: int = 256,
            reset_interval: int = 256_000,
            history_lengths: Optional[List[int]] = None,
            tag_widths: Optional[List[int]] = None,
            # SC parameters
            sc_num_tables: int = 6,
            sc_table_size: int = 512,
            sc_history_lengths: Optional[List[int]] = None,
            sc_counter_bits: int = 3,
            sc_bias_table_size: int = 256,
            sc_bias_counter_bits: int = 6,
            sc_initial_threshold: int = 35,
            sc_per_pc_threshold_bits: int = 6,
            # ITTAGE parameters
            ittage_num_banks: int = 8,
            ittage_table_size: int = 256,
            ittage_history_lengths: Optional[List[int]] = None,
            ittage_tag_widths: Optional[List[int]] = None,
            ittage_reset_interval: int = 256_000,
        ):
            self.num_banks = num_banks
            self.table_size = table_size
            self.loop_table_size = loop_table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths
                if history_lengths is not None
                else [5, 11, 22, 44, 89, 178, 356, 712]
            )
            self.tag_widths = (
                tag_widths
                if tag_widths is not None
                else [8, 8, 9, 9, 10, 10, 11, 11]
            )
            self.sc_num_tables = sc_num_tables
            self.sc_table_size = sc_table_size
            self.sc_history_lengths = (
                sc_history_lengths
                if sc_history_lengths is not None
                else [0, 2, 4, 8, 12, 16]
            )
            self.sc_counter_bits = sc_counter_bits
            self.sc_bias_table_size = sc_bias_table_size
            self.sc_bias_counter_bits = sc_bias_counter_bits
            self.sc_initial_threshold = sc_initial_threshold
            self.sc_per_pc_threshold_bits = sc_per_pc_threshold_bits
            self.ittage_num_banks = ittage_num_banks
            self.ittage_table_size = ittage_table_size
            self.ittage_history_lengths = (
                ittage_history_lengths
                if ittage_history_lengths is not None
                else [4, 8, 16, 32, 64, 128, 256, 512]
            )
            self.ittage_tag_widths = (
                ittage_tag_widths
                if ittage_tag_widths is not None
                else [9, 9, 10, 10, 11, 11, 12, 12]
            )
            self.ittage_reset_interval = ittage_reset_interval
            _validate_history_lengths(
                self.history_lengths, "ScLTage history_lengths"
            )
            _validate_history_lengths(
                self.ittage_history_lengths, "ScLTage ittage_history_lengths"
            )

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.ScLTage(num_banks={self.num_banks}, "
                f"table_size={self.table_size}, "
                f"sc_num_tables={self.sc_num_tables}, "
                f"ittage_num_banks={self.ittage_num_banks})"
            )


# ── Memory Dependence Predictor ──────────────────────────────────────────────


class MemDepPredictor:
    """Namespace for memory dependence predictor configurations."""

    class Blind:
        def __repr__(self) -> str:
            return "MemDepPredictor.Blind()"

    class StoreSet:
        def __init__(self, ssit_size: int = 2048, lfst_size: int = 256):
            self.ssit_size = ssit_size
            self.lfst_size = lfst_size

        def __repr__(self) -> str:
            return (
                f"MemDepPredictor.StoreSet(ssit_size={self.ssit_size}, "
                f"lfst_size={self.lfst_size})"
            )


# ── Replacement Policy ───────────────────────────────────────────────────────


class ReplacementPolicy:
    """Namespace for cache replacement policies."""

    class LRU:
        def __repr__(self) -> str:
            return "ReplacementPolicy.LRU()"

    class PLRU:
        def __repr__(self) -> str:
            return "ReplacementPolicy.PLRU()"

    class FIFO:
        def __repr__(self) -> str:
            return "ReplacementPolicy.FIFO()"

    class Random:
        def __repr__(self) -> str:
            return "ReplacementPolicy.Random()"

    class MRU:
        def __repr__(self) -> str:
            return "ReplacementPolicy.MRU()"


# ── Prefetcher ───────────────────────────────────────────────────────────────


class Prefetcher:
    """Namespace for prefetcher configurations."""

    class Off:
        def __repr__(self) -> str:
            return "Prefetcher.Off()"

    class NextLine:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.NextLine(degree={self.degree})"

    class Stride:
        def __init__(self, degree: int = 1, table_size: int = 64):
            self.degree = degree
            self.table_size = table_size

        def __repr__(self) -> str:
            return (
                f"Prefetcher.Stride(degree={self.degree}, table_size={self.table_size})"
            )

    class Stream:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.Stream(degree={self.degree})"

    class Tagged:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.Tagged(degree={self.degree})"


# ── Memory Controller ────────────────────────────────────────────────────────


class MemoryController:
    """Namespace for memory controller configurations."""

    class Simple:
        def __repr__(self) -> str:
            return "MemoryController.Simple()"

    class DRAM:
        def __init__(
            self,
            t_cas: int = 14,
            t_ras: int = 14,
            t_pre: int = 14,
            row_miss_latency: int = 120,
        ):
            self.t_cas = t_cas
            self.t_ras = t_ras
            self.t_pre = t_pre
            self.row_miss_latency = row_miss_latency

        def __repr__(self) -> str:
            return (
                f"MemoryController.DRAM(t_cas={self.t_cas}, t_ras={self.t_ras}, "
                f"t_pre={self.t_pre}, row_miss_latency={self.row_miss_latency})"
            )


# ── Functional Units ──────────────────────────────────────────────────────────


class Fu:
    """Functional unit pool configuration for the O3 backend.

    Instantiate ``Fu`` with a list of unit descriptors (the inner classes).
    Any FU type omitted will be absent from the pool, so include every type
    your workload exercises::

        Fu([
            Fu.IntAlu(count=4, latency=1),
            Fu.IntMul(count=1, latency=3),
            Fu.IntDiv(count=1, latency=35),
            Fu.FpAdd(count=2, latency=4),
            Fu.FpMul(count=2, latency=5),
            Fu.FpFma(count=2, latency=5),
            Fu.FpDivSqrt(count=1, latency=21),
            Fu.Branch(count=2, latency=1),
            Fu.Mem(count=2, latency=1),
        ])
    """

    class IntAlu:
        """Integer ALU: add, sub, logic, shift, compare, set-less-than."""

        def __init__(self, count: int = 4, latency: int = 1):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.IntAlu(count={self.count}, latency={self.latency})"

    class IntMul:
        """Integer multiplier: mul, mulh, mulhsu, mulhu."""

        def __init__(self, count: int = 1, latency: int = 3):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.IntMul(count={self.count}, latency={self.latency})"

    class IntDiv:
        """Integer divider: div, divu, rem, remu. Non-pipelined."""

        def __init__(self, count: int = 1, latency: int = 35):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.IntDiv(count={self.count}, latency={self.latency})"

    class FpAdd:
        """FP adder: fadd, fsub, fmin, fmax, fcmp, fcvt."""

        def __init__(self, count: int = 2, latency: int = 4):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpAdd(count={self.count}, latency={self.latency})"

    class FpMul:
        """FP multiplier: fmul."""

        def __init__(self, count: int = 2, latency: int = 5):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpMul(count={self.count}, latency={self.latency})"

    class FpFma:
        """FP fused multiply-add: fmadd, fmsub, fnmadd, fnmsub."""

        def __init__(self, count: int = 2, latency: int = 5):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpFma(count={self.count}, latency={self.latency})"

    class FpDivSqrt:
        """FP divider/sqrt: fdiv, fsqrt. Non-pipelined."""

        def __init__(self, count: int = 1, latency: int = 21):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpDivSqrt(count={self.count}, latency={self.latency})"

    class Branch:
        """Branch/jump unit: all conditional branches, jal, jalr."""

        def __init__(self, count: int = 2, latency: int = 1):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.Branch(count={self.count}, latency={self.latency})"

    class Mem:
        """Memory address calculation for loads and stores."""

        def __init__(self, count: int = 2, latency: int = 1):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.Mem(count={self.count}, latency={self.latency})"

    # ── Vector FU classes (factory-generated) ──────────────────────────────

    @staticmethod
    def _make_vec_fu(name, doc, default_count, default_latency):
        def init(self, count=default_count, latency=default_latency):
            self.count = count
            self.latency = latency

        def repr_(self):
            return f"Fu.{name}(count={self.count}, latency={self.latency})"

        return type(name, (), {"__init__": init, "__repr__": repr_, "__doc__": doc})

    # Default pool matching Skylake-class hardware
    _DEFAULTS: "list"

    def __init__(self, units=None):
        self.units = list(units) if units is not None else list(Fu._DEFAULTS)

    def __repr__(self) -> str:
        inner = ", ".join(repr(u) for u in self.units)
        return f"Fu([{inner}])"


# Attach vector FU classes to Fu namespace
for _name, _doc, _count, _lat in [
    ("VecIntAlu", "Vector integer ALU", 1, 1),
    ("VecIntMul", "Vector integer multiplier", 1, 3),
    ("VecIntDiv", "Vector integer divider", 1, 20),
    ("VecFpAlu", "Vector FP ALU", 1, 4),
    ("VecFpFma", "Vector FP FMA", 1, 5),
    ("VecFpDivSqrt", "Vector FP div/sqrt", 1, 20),
    ("VecMem", "Vector memory unit", 1, 1),
    ("VecPermute", "Vector permute unit", 1, 1),
]:
    setattr(Fu, _name, Fu._make_vec_fu(_name, _doc, _count, _lat))

Fu._DEFAULTS = [
    Fu.IntAlu(count=4, latency=1),
    Fu.IntMul(count=1, latency=3),
    Fu.IntDiv(count=1, latency=35),
    Fu.FpAdd(count=2, latency=4),
    Fu.FpMul(count=2, latency=5),
    Fu.FpFma(count=2, latency=5),
    Fu.FpDivSqrt(count=1, latency=21),
    Fu.Branch(count=2, latency=1),
    Fu.Mem(count=2, latency=1),
    Fu.VecIntAlu(count=1, latency=1),
    Fu.VecIntMul(count=1, latency=3),
    Fu.VecIntDiv(count=1, latency=20),
    Fu.VecFpAlu(count=1, latency=4),
    Fu.VecFpFma(count=1, latency=5),
    Fu.VecFpDivSqrt(count=1, latency=20),
    Fu.VecMem(count=1, latency=1),
    Fu.VecPermute(count=1, latency=1),
]


# ── Backend ──────────────────────────────────────────────────────────────────


class Backend:
    """Namespace for pipeline backend configurations."""

    class InOrder:
        def __repr__(self) -> str:
            return "Backend.InOrder()"

    class OutOfOrder:
        def __init__(
            self,
            rob_size: int = 128,
            store_buffer_size: int = 32,
            issue_queue_size: int = 32,
            load_queue_size: int = 32,
            load_ports: int = 2,
            store_ports: int = 1,
            prf_gpr_size: int = 256,
            prf_fpr_size: int = 128,
            fu_config=None,
            checkpoint_count: int = 0,
            prf_vpr_size: int = 64,
            vec_chaining: bool = True,
        ):
            self.rob_size = rob_size
            self.store_buffer_size = store_buffer_size
            self.issue_queue_size = issue_queue_size
            self.load_queue_size = load_queue_size
            self.load_ports = load_ports
            self.store_ports = store_ports
            self.prf_gpr_size = prf_gpr_size
            self.prf_fpr_size = prf_fpr_size
            self.fu_config = fu_config if fu_config is not None else Fu()
            self.checkpoint_count = checkpoint_count
            self.prf_vpr_size = prf_vpr_size
            self.vec_chaining = vec_chaining

        def __repr__(self) -> str:
            return (
                f"Backend.OutOfOrder(rob_size={self.rob_size}, "
                f"store_buffer_size={self.store_buffer_size}, "
                f"issue_queue_size={self.issue_queue_size}, "
                f"load_queue_size={self.load_queue_size}, "
                f"load_ports={self.load_ports}, "
                f"store_ports={self.store_ports})"
            )


# ── Cache ────────────────────────────────────────────────────────────────────


class Cache:
    """Single cache level configuration."""

    class NINE:
        """No Inclusion, Non-Exclusive (default)."""

        def __repr__(self) -> str:
            return "Cache.NINE()"

    class Inclusive:
        """Inclusive: L2 eviction back-invalidates matching L1 lines."""

        def __repr__(self) -> str:
            return "Cache.Inclusive()"

    class Exclusive:
        """Exclusive: L1 eviction installs line into L2 (swap)."""

        def __repr__(self) -> str:
            return "Cache.Exclusive()"

    def __init__(
        self,
        size: "str | int" = "4KB",
        line: "str | int" = "64B",
        ways: int = 1,
        policy: "ReplacementPolicy.LRU | ReplacementPolicy.PLRU | ReplacementPolicy.FIFO | ReplacementPolicy.Random | ReplacementPolicy.MRU | None" = None,
        latency: int = 1,
        prefetcher: "Prefetcher.Off | Prefetcher.NextLine | Prefetcher.Stride | Prefetcher.Stream | Prefetcher.Tagged | None" = None,
        mshr_count: int = 0,
    ):
        self.size_bytes = _parse_size(size)
        self.line_bytes = _parse_size(line)
        self.ways = ways
        self.policy = policy if policy is not None else ReplacementPolicy.LRU()
        self.latency = latency
        self.prefetcher = prefetcher if prefetcher is not None else Prefetcher.Off()
        self.mshr_count = mshr_count

    def __repr__(self) -> str:
        return (
            f"Cache(size={self.size_bytes}, line={self.line_bytes}, "
            f"ways={self.ways}, policy={self.policy!r}, "
            f"latency={self.latency}, prefetcher={self.prefetcher!r})"
        )


# Disabled cache dict for levels set to None
_DISABLED_CACHE_DICT: Dict[str, Any] = {
    "enabled": False,
    "size_bytes": 4096,
    "line_bytes": 64,
    "ways": 1,
    "policy": "LRU",
    "latency": 1,
    "prefetcher": "None",
    "prefetch_table_size": 0,
    "prefetch_degree": 0,
}

_DISABLED_CACHE_DICT_ZERO: Dict[str, Any] = {
    "enabled": False,
    "size_bytes": 0,
    "line_bytes": 0,
    "ways": 0,
    "policy": "LRU",
    "latency": 0,
    "prefetcher": "None",
    "prefetch_table_size": 0,
    "prefetch_degree": 0,
}
