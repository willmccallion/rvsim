"""
Namespace types for simulator configuration.

Provides structured, Pythonic alternatives to raw string enums:
- BranchPredictor: Static, GShare, TAGE, Perceptron, Tournament
- ReplacementPolicy: LRU, PLRU, FIFO, Random, MRU
- Prefetcher: None_, NextLine, Stride, Stream, Tagged
- MemoryController: Simple, DRAM
- Backend: InOrder, OutOfOrder
- Cache: cache level configuration with size parsing
"""

from __future__ import annotations

import re
from typing import Any, Dict, List, Optional


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


# ── Branch Predictor ─────────────────────────────────────────────────────────


class BranchPredictor:
    """Namespace for branch predictor configurations."""

    class Static:
        def _to_dict_value(self) -> str:
            return "Static"

        def _sub_dict(self) -> dict:
            return {}

        def __repr__(self) -> str:
            return "BranchPredictor.Static()"

    class GShare:
        def _to_dict_value(self) -> str:
            return "GShare"

        def _sub_dict(self) -> dict:
            return {}

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

        def _to_dict_value(self) -> str:
            return "TAGE"

        def _sub_dict(self) -> dict:
            return {
                "num_banks": self.num_banks,
                "table_size": self.table_size,
                "loop_table_size": self.loop_table_size,
                "reset_interval": self.reset_interval,
                "history_lengths": self.history_lengths,
                "tag_widths": self.tag_widths,
            }

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

        def _to_dict_value(self) -> str:
            return "Perceptron"

        def _sub_dict(self) -> dict:
            return {
                "history_length": self.history_length,
                "table_bits": self.table_bits,
            }

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

        def _to_dict_value(self) -> str:
            return "Tournament"

        def _sub_dict(self) -> dict:
            return {
                "global_size_bits": self.global_size_bits,
                "local_hist_bits": self.local_hist_bits,
                "local_pred_bits": self.local_pred_bits,
            }

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
        def _to_dict_value(self) -> str:
            return "LRU"

        def __repr__(self) -> str:
            return "ReplacementPolicy.LRU()"

    class PLRU:
        def _to_dict_value(self) -> str:
            return "PLRU"

        def __repr__(self) -> str:
            return "ReplacementPolicy.PLRU()"

    class FIFO:
        def _to_dict_value(self) -> str:
            return "FIFO"

        def __repr__(self) -> str:
            return "ReplacementPolicy.FIFO()"

    class Random:
        def _to_dict_value(self) -> str:
            return "Random"

        def __repr__(self) -> str:
            return "ReplacementPolicy.Random()"

    class MRU:
        def _to_dict_value(self) -> str:
            return "MRU"

        def __repr__(self) -> str:
            return "ReplacementPolicy.MRU()"


# ── Prefetcher ───────────────────────────────────────────────────────────────


class Prefetcher:
    """Namespace for prefetcher configurations."""

    class None_:
        def _to_dict_value(self) -> str:
            return "None"

        def _degree(self) -> int:
            return 0

        def _table_size(self) -> int:
            return 0

        def __repr__(self) -> str:
            return "Prefetcher.None_()"

    class NextLine:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def _to_dict_value(self) -> str:
            return "NextLine"

        def _degree(self) -> int:
            return self.degree

        def _table_size(self) -> int:
            return 0

        def __repr__(self) -> str:
            return f"Prefetcher.NextLine(degree={self.degree})"

    class Stride:
        def __init__(self, degree: int = 1, table_size: int = 64):
            self.degree = degree
            self.table_size = table_size

        def _to_dict_value(self) -> str:
            return "Stride"

        def _degree(self) -> int:
            return self.degree

        def _table_size(self) -> int:
            return self.table_size

        def __repr__(self) -> str:
            return (
                f"Prefetcher.Stride(degree={self.degree}, table_size={self.table_size})"
            )

    class Stream:
        def _to_dict_value(self) -> str:
            return "Stream"

        def _degree(self) -> int:
            return 0

        def _table_size(self) -> int:
            return 0

        def __repr__(self) -> str:
            return "Prefetcher.Stream()"

    class Tagged:
        def _to_dict_value(self) -> str:
            return "Tagged"

        def _degree(self) -> int:
            return 0

        def _table_size(self) -> int:
            return 0

        def __repr__(self) -> str:
            return "Prefetcher.Tagged()"


# ── Memory Controller ────────────────────────────────────────────────────────


class MemoryController:
    """Namespace for memory controller configurations."""

    class Simple:
        def _to_dict_value(self) -> str:
            return "Simple"

        def _sub_dict(self) -> dict:
            return {}

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

        def _to_dict_value(self) -> str:
            return "Dram"

        def _sub_dict(self) -> dict:
            return {
                "t_cas": self.t_cas,
                "t_ras": self.t_ras,
                "t_pre": self.t_pre,
                "row_miss_latency": self.row_miss_latency,
            }

        def __repr__(self) -> str:
            return (
                f"MemoryController.DRAM(t_cas={self.t_cas}, t_ras={self.t_ras}, "
                f"t_pre={self.t_pre}, row_miss_latency={self.row_miss_latency})"
            )


# ── Backend ──────────────────────────────────────────────────────────────────


class Backend:
    """Namespace for pipeline backend configurations."""

    class InOrder:
        def _to_dict_value(self) -> str:
            return "InOrder"

        def _rob_size(self) -> int:
            return 64

        def _store_buffer_size(self) -> int:
            return 16

        def __repr__(self) -> str:
            return "Backend.InOrder()"

    class OutOfOrder:
        def __init__(self, rob_size: int = 64, store_buffer_size: int = 16):
            self.rob_size = rob_size
            self.store_buffer_size = store_buffer_size

        def _to_dict_value(self) -> str:
            return "OutOfOrder"

        def _rob_size(self) -> int:
            return self.rob_size

        def _store_buffer_size(self) -> int:
            return self.store_buffer_size

        def __repr__(self) -> str:
            return (
                f"Backend.OutOfOrder(rob_size={self.rob_size}, "
                f"store_buffer_size={self.store_buffer_size})"
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
        self.prefetcher = prefetcher if prefetcher is not None else Prefetcher.None_()

    def _to_cache_dict(self) -> Dict[str, Any]:
        return {
            "enabled": True,
            "size_bytes": self.size_bytes,
            "line_bytes": self.line_bytes,
            "ways": self.ways,
            "policy": self.policy._to_dict_value(),
            "latency": self.latency,
            "prefetcher": self.prefetcher._to_dict_value(),
            "prefetch_table_size": self.prefetcher._table_size(),
            "prefetch_degree": self.prefetcher._degree(),
        }

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
