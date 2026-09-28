"""The machines both simulators run in the comparison, as plain data.

`run_rvsim.py` and gem5's `gem5_single.py` each build their simulator's
configuration from one of these, so a variant describes the same machine on
both sides. Every variant starts from the P550-like base and changes one
thing: the branch predictor, the cache sizes, the L1D's MSHRs, or a
prefetcher both simulators implement the same way.
"""

import copy

CLOCK_MHZ = 1400

# gem5's RISC-V default vector length, and one vector unit per issue slot:
# gem5's SIMD_Unit pool, which rvsim splits by operation class.
VLEN = 256
VECTOR_UNITS = 3

BASE = {
    "bp": {"kind": "tournament", "global_bits": 13, "local_hist_bits": 11, "local_pred_bits": 11},
    # Hit latency, and the latency from a fill to answering its requests
    # (gem5's tag/data and response latencies).
    "l1i": {"size_kb": 32, "assoc": 8, "mshrs": 16, "tgts": 20, "latency": 1, "response": 1},
    "l1d": {"size_kb": 32, "assoc": 8, "mshrs": 16, "tgts": 20, "latency": 1, "response": 1},
    "l2": {"size_kb": 256, "assoc": 16, "mshrs": 20, "tgts": 12, "latency": 10, "response": 1},
    # The bus between the L2 and memory, each way, and memory itself: a
    # fixed latency and a bandwidth (gem5's SimpleMemory, rvsim's Simple).
    "bus": {"width_bytes": 8, "latency": 4},
    # rvsim's Simple controller has a fixed 120-cycle latency.
    "memory": {"latency": 120, "bandwidth_gib_s": 12.8},
    # Next-N-line prefetching on a miss or a hit to a prefetched line:
    # gem5's TaggedPrefetcher and rvsim's Tagged prefetcher.
    "l1d_prefetch_degree": None,
}

# gem5's TAGE with rvsim's table shape: eight tagged tables of 2048 entries
# over a 2048-entry bimodal, 3-bit counters, 2-bit useful bits, a useful
# reset every 2^18 updates, and gem5's geometric history lengths.
TAGE = {
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


def _with(**changes) -> dict:
    variant = copy.deepcopy(BASE)
    for key, value in changes.items():
        if key != "bp" and isinstance(value, dict):
            variant[key].update(value)
        else:
            variant[key] = value
    return variant


VARIANTS = {
    "base": copy.deepcopy(BASE),
    "bp_tage": _with(bp=TAGE),
    "bp_small_tournament": _with(
        bp={"kind": "tournament", "global_bits": 10, "local_hist_bits": 8, "local_pred_bits": 8}
    ),
    "caches_small": _with(
        l1i={"size_kb": 8, "assoc": 4},
        l1d={"size_kb": 8, "assoc": 4},
        l2={"size_kb": 64, "assoc": 8},
    ),
    "caches_large": _with(
        l1i={"size_kb": 64},
        l1d={"size_kb": 64},
        l2={"size_kb": 1024},
    ),
    "l1d_one_mshr": _with(l1d={"mshrs": 1}),
    "l1d_prefetch": _with(l1d_prefetch_degree=2),
}
