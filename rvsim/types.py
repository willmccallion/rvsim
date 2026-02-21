"""
Namespace types for simulator configuration.

Provides structured, Pythonic alternatives to raw string enums:
- BranchPredictor: Static, GShare, TAGE, Perceptron, Tournament
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
            num_banks: int = 4,
            table_size: int = 2048,
            loop_table_size: int = 256,
            reset_interval: int = 2000,
            history_lengths: Optional[List[int]] = None,
            tag_widths: Optional[List[int]] = None,
        ):
            self.num_banks = num_banks
            self.table_size = table_size
            self.loop_table_size = loop_table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths if history_lengths is not None else [5, 15, 44, 130]
            )
            self.tag_widths = tag_widths if tag_widths is not None else [9, 9, 10, 10]

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
        def __repr__(self) -> str:
            return "Prefetcher.Stream()"

    class Tagged:
        def __repr__(self) -> str:
            return "Prefetcher.Tagged()"


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
        ):
            self.rob_size = rob_size
            self.store_buffer_size = store_buffer_size
            self.issue_queue_size = issue_queue_size

        def __repr__(self) -> str:
            return (
                f"Backend.OutOfOrder(rob_size={self.rob_size}, "
                f"store_buffer_size={self.store_buffer_size}, "
                f"issue_queue_size={self.issue_queue_size})"
            )


# ── Cache ────────────────────────────────────────────────────────────────────


class Cache:
    """Single cache level configuration."""

    def __init__(
        self,
        size="4KB",
        line="64B",
        ways: int = 1,
        policy=None,
        latency: int = 1,
        prefetcher=None,
    ):
        self.size_bytes = _parse_size(size)
        self.line_bytes = _parse_size(line)
        self.ways = ways
        self.policy = policy if policy is not None else ReplacementPolicy.LRU()
        self.latency = latency
        self.prefetcher = prefetcher if prefetcher is not None else Prefetcher.Off()

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
