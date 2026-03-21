# Branch Prediction

rvsim implements five pluggable branch predictors with shared infrastructure. The predictor is consulted during Fetch1 to steer the instruction stream speculatively.

## Shared Infrastructure

All predictors share these components:

### Branch Target Buffer (BTB)

Set-associative cache (default: 4096 entries, 4-way) that maps branch PCs to their target addresses. Used for indirect jumps where the target isn't encoded in the instruction.

### Return Address Stack (RAS)

Circular buffer (default: 32 entries) for call/return prediction.

Per RISC-V spec Table 2.1, both **x1 (ra)** and **x5 (t0)** are recognized as link registers:

| Instruction | rd is link? | rs1 is link? | Action |
|-------------|-------------|--------------|--------|
| `jal rd, offset` | Yes | — | **Push** return address onto RAS |
| `jal rd, offset` | No | — | No RAS action (plain jump) |
| `jalr rd, rs1, offset` | No | Yes | **Pop** from RAS (return) |
| `jalr rd, rs1, offset` | Yes | Yes, rd ≠ rs1 | **Pop then push** (coroutine swap) |
| `jalr rd, rs1, offset` | Yes | Yes, rd = rs1 | **Push** (call through link register) |
| `jalr rd, rs1, offset` | Yes | No | **Push** (indirect call) |

The RAS supports speculative recovery: on a branch misprediction, the RAS pointer is restored from the per-instruction snapshot.

### Global History Register (GHR)

Arbitrary-length bit vector recording the direction (taken/not-taken) of recent branches. The GHR is speculatively updated during Fetch1 and repaired on misprediction from per-instruction snapshots.

The GHR length is unlimited — it grows to match the longest history needed by the selected predictor (e.g., TAGE's geometric history lengths can exceed 700 bits).

## Predictors

### Static

Always predicts not-taken. Useful as a baseline for measuring how much a predictor contributes.

### GShare

XOR of the branch PC and the global history register indexes into a table of 2-bit saturating counters. Simple and effective for workloads with strong global correlation.

### Tournament

Two-level adaptive predictor with three components:

1. **Global predictor** — 2-bit counters indexed by global history
2. **Local predictor** — per-PC local history table feeding a second table of 2-bit counters
3. **Meta-predictor (chooser)** — selects between global and local predictions based on which has been more accurate recently

Configurable parameters: `global_size_bits`, `local_hist_bits`, `local_pred_bits`.

### Perceptron

Neural branch predictor. Each entry in the table is a vector of integer weights, one per GHR bit. The dot product of the weight vector and the recent branch history determines the prediction. Weights are trained on mispredictions using a threshold-based update rule.

Configurable parameters: `history_length`, `table_bits`.

### TAGE (Tagged Geometric History Length)

The most accurate predictor available. Uses multiple tagged tables with geometrically increasing history lengths:

- **Base predictor** — simple bimodal table (always consulted)
- **Tagged tables** — each table uses a different history length (default: 5, 15, 44, 130). Entries are tagged with a hash of the PC and history to avoid aliasing.
- **Longest match wins** — the prediction comes from the table with the longest matching history
- **Loop predictor** — detects counted loops and predicts the loop exit iteration
- **Useful counter reset** — periodically resets the "useful" counters to allow new entries to replace stale ones

Configurable parameters: `num_banks`, `table_size`, `loop_table_size`, `reset_interval`, `history_lengths`, `tag_widths`.

## Predictor Comparison

Here's a representative comparison on the included benchmarks (width=1, default caches):

| Predictor | Accuracy (geomean) | IPC (geomean) | Speedup vs Static |
|-----------|-------------------|---------------|-------------------|
| Static | 34.4% | 0.55 | 1.00× |
| GShare | 60.1% | 0.62 | 1.07× |
| Perceptron | 64.1% | 0.64 | 1.06× |
| Tournament | 71.4% | 0.67 | 1.21× |
| TAGE | 85.0% | 0.75 | 1.30× |

TAGE provides the best accuracy and performance across all workloads, with particularly strong results on branch-heavy programs like `qsort` (83% accuracy vs GShare's 60%).
