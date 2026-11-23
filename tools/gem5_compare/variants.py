"""The machines both simulators run in the comparison, as plain data.

`run_rvsim.py` and gem5's `gem5_single.py` each build their simulator's
configuration from one of these, so a variant describes the same machine on
both sides. Every variant starts from the P550-like base and changes one
thing: the branch predictor, the cache sizes, the L1D's MSHRs, or a
prefetcher both simulators implement the same way.
"""

from typing import Literal, TypedDict

CLOCK_MHZ = 1400

# gem5's RISC-V default vector length, and one vector unit per issue slot:
# gem5's SIMD_Unit pool, which rvsim splits by operation class.
VLEN = 256
VECTOR_UNITS = 3


class Tournament(TypedDict):
    """gem5's TournamentBP and rvsim's Tournament, sized in bits."""

    kind: Literal["tournament"]
    global_bits: int
    local_hist_bits: int
    local_pred_bits: int


class Tage(TypedDict):
    """gem5's TAGE in rvsim's table shape."""

    kind: Literal["tage"]
    tables: int
    log_table_size: int
    tag_widths: list[int]
    min_hist: int
    max_hist: int
    log_u_reset: int


class TageScL(TypedDict):
    """Each simulator's TAGE-SC-L at its own configuration."""

    kind: Literal["tage_sc_l"]


BranchPredictor = Tournament | Tage | TageScL


class Cache(TypedDict):
    """A cache's geometry and its hit and response latencies."""

    size_kb: int
    assoc: int
    mshrs: int
    tgts: int
    latency: int
    response: int


class Bus(TypedDict):
    width_bytes: int
    latency: int


class Memory(TypedDict):
    latency: int
    bandwidth_gib_s: float


class StridePrefetch(TypedDict):
    degree: int
    table_size: int


class Variant(TypedDict):
    """One machine, as both simulators build it."""

    bp: BranchPredictor
    l1i: Cache
    l1d: Cache
    l2: Cache
    bus: Bus
    memory: Memory
    l1d_prefetch_degree: int | None
    l1d_stride_prefetch: StridePrefetch | None


BASE: Variant = {
    "bp": {
        "kind": "tournament",
        "global_bits": 13,
        "local_hist_bits": 11,
        "local_pred_bits": 11,
    },
    # Hit latency, and the latency from a fill to answering its requests
    # (gem5's tag/data and response latencies).
    "l1i": {
        "size_kb": 32,
        "assoc": 8,
        "mshrs": 16,
        "tgts": 20,
        "latency": 1,
        "response": 1,
    },
    "l1d": {
        "size_kb": 32,
        "assoc": 8,
        "mshrs": 16,
        "tgts": 20,
        "latency": 1,
        "response": 1,
    },
    "l2": {
        "size_kb": 256,
        "assoc": 16,
        "mshrs": 20,
        "tgts": 12,
        "latency": 10,
        "response": 1,
    },
    # The bus between the L2 and memory, each way, and memory itself: a
    # fixed latency and a bandwidth (gem5's SimpleMemory, rvsim's Simple).
    "bus": {"width_bytes": 8, "latency": 4},
    "memory": {"latency": 120, "bandwidth_gib_s": 12.8},
    # Next-N-line prefetching on a miss or a hit to a prefetched line:
    # gem5's TaggedPrefetcher and rvsim's Tagged prefetcher.
    "l1d_prefetch_degree": None,
    # A PC-indexed stride prefetcher on the L1D trained on every load:
    # gem5's StridePrefetcher and rvsim's Stride, with gem5's confidence
    # set to rvsim's (a 2-bit counter from zero that must saturate).
    "l1d_stride_prefetch": None,
}

# gem5's TAGE with rvsim's table shape: eight tagged tables of 2048 entries
# over a 2048-entry bimodal sharing a hysteresis bit between four entries
# (gem5's default), 3-bit counters, 2-bit useful bits, a useful
# reset every 2^18 updates, and gem5's geometric history lengths.
TAGE: Tage = {
    "kind": "tage",
    "tables": 8,
    "log_table_size": 11,
    "tag_widths": [8, 8, 9, 9, 10, 10, 11, 11],
    "min_hist": 5,
    "max_hist": 712,
    "log_u_reset": 18,
}


def tage_history_lengths(min_hist: int, max_hist: int, tables: int) -> list[int]:
    """The history length of each tagged table, as gem5's `TAGEBase` computes
    them (`calculateParameters`)."""
    lengths = [min_hist]
    for i in range(2, tables):
        ratio = (max_hist / min_hist) ** ((i - 1) / (tables - 1))
        lengths.append(int(min_hist * ratio + 0.5))
    lengths.append(max_hist)
    return lengths


VARIANTS: dict[str, Variant] = {
    "base": BASE,
    "bp_tage": {**BASE, "bp": TAGE},
    # Each simulator's TAGE-SC-L at its own configuration: rvsim's SC-L-TAGE
    # defaults against gem5's 64KB CBP-5 TAGE-SC-L.
    "bp_tage_sc_l": {**BASE, "bp": {"kind": "tage_sc_l"}},
    "bp_small_tournament": {
        **BASE,
        "bp": {
            "kind": "tournament",
            "global_bits": 10,
            "local_hist_bits": 8,
            "local_pred_bits": 8,
        },
    },
    "caches_small": {
        **BASE,
        "l1i": {**BASE["l1i"], "size_kb": 8, "assoc": 4},
        "l1d": {**BASE["l1d"], "size_kb": 8, "assoc": 4},
        "l2": {**BASE["l2"], "size_kb": 64, "assoc": 8},
    },
    "caches_large": {
        **BASE,
        "l1i": {**BASE["l1i"], "size_kb": 64},
        "l1d": {**BASE["l1d"], "size_kb": 64},
        "l2": {**BASE["l2"], "size_kb": 1024},
    },
    "l1d_one_mshr": {**BASE, "l1d": {**BASE["l1d"], "mshrs": 1}},
    "l1d_prefetch": {**BASE, "l1d_prefetch_degree": 2},
    "l1d_stride_prefetch": {
        **BASE,
        "l1d_stride_prefetch": {"degree": 4, "table_size": 64},
    },
}
