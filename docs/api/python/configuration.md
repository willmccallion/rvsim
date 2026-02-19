# Configuration (Python)

How to configure the simulator from Python: config schema, parameters, and mapping to Rust.

**Source:** `rvsim/config.py`, `rvsim/core/params.py`.

---

## Overview

Configuration is **Python-first**: the library provides a **`SimConfig`** root object. You define your machine in a script (e.g., `scripts/p550/config.py`) by starting from `SimConfig.default()` or `SimConfig.minimal()` and setting fields. The config is converted to a dict and passed to the [Rust bindings](../rust/bindings.md) to build the hardware `Config`.

---

## SimConfig Schema (`config.py`)

### `SimConfig` root

- **`general`**: `trace_instructions`, `start_pc`, `direct_mode` (True for bare-metal, False for OS), `initial_sp`.
- **`system`**: Address map: `ram_base`, `uart_base`, `disk_base`, `clint_base`, `syscon_base`. Also `bus_width` and `bus_latency`.
- **`memory`**: `ram_size`, `controller` (`"Simple"` or `"Dram"`), timing (`t_cas`, `t_ras`, `t_pre`, `row_miss_latency`), `tlb_size`.
- **`cache`**: Hierarchy of `CacheConfig` for `l1_i`, `l1_d`, `l2`, `l3`.
- **`pipeline`**: `width`, `branch_predictor` (`"TAGE"`, `"Perceptron"`, `"Tournament"`, `"GShare"`, `"Static"`), `btb_size`, `ras_size`, and predictor-specific configs.

### Cache configuration (`CacheConfig`)

- **`enabled`**: bool.
- **`size_bytes`, `line_bytes`, `ways`**: capacity and associativity.
- **`policy`**: `"LRU"`, `"PLRU"`, `"FIFO"`, `"Random"`, `"MRU"`. See [replacement policies](../../architecture/memory_hierarchy.md#replacement-policies).
- **`latency`**: access latency in cycles.
- **`prefetcher`**: `"None"`, `"NextLine"`, `"Stride"`, `"Stream"`, `"Tagged"`.
- **`prefetch_degree`, `prefetch_table_size`**: prefetch parameters.

### Branch Predictor configurations

- **`TageConfig`**: `num_banks`, `table_size`, `loop_table_size`, `reset_interval`, `history_lengths` (List), `tag_widths` (List).
- **`PerceptronConfig`**: `history_length`, `table_bits`.
- **`TournamentConfig`**: `global_size_bits`, `local_hist_bits`, `local_pred_bits`.

---

## Factory methods

- **`SimConfig.default()`**: 1-wide, static BP, caches disabled. RAM size 0x1000_0000.
- **`SimConfig.minimal()`**: Fast config for debugging; minimal sizes, no caches, static BP.

---

## Usage in scripts

Machine scripts return a `SimConfig` (or a function that returns one). For example, `scripts/p550/config.py`:

```python
from rvsim import SimConfig, TageConfig

def p550_config(branch_predictor="TAGE"):
    c = SimConfig.default()
    c.pipeline.width = 3
    c.pipeline.branch_predictor = branch_predictor
    c.cache.l1_i.enabled = True
    c.cache.l1_i.size_bytes = 32768
    # ... more settings ...
    return c

# Entry point for Simulator.config()
config = p550_config
```

The [Simulator loader](../rust/bindings.md) expects the config module to expose a **`config`** attribute (either the `SimConfig` object itself or a function returning one).

---

## See also

- [Rust bindings](../rust/bindings.md) — config conversion.
- [Scripting](scripting.md) — using configs in scripts.
- [Architecture docs](../../architecture/README.md) — implementation details.
