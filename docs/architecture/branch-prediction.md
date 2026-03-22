# Branch Prediction

rvsim implements six pluggable branch predictors with shared infrastructure. The predictor is consulted during Fetch1 to steer the instruction stream speculatively.

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

Uses multiple tagged tables with geometrically increasing history lengths:

- **Base predictor** — simple bimodal table (always consulted)
- **Tagged tables** — each table uses a different history length (default: 5, 11, 22, 44, 89, 178, 356, 712 for 8 banks). Entries are tagged with a hash of the PC and history to avoid aliasing.
- **Longest match wins** — the prediction comes from the table with the longest matching history
- **Loop predictor** — detects counted loops and predicts the loop exit iteration
- **USE_ALT_ON_NA** — meta-counter that learns whether newly allocated (weak) provider entries should be trusted or whether the alternate (second-longest match) prediction is better. When the provider entry's counter is weak (0 or -1) and the meta-counter is non-negative, the alternate prediction is used instead.
- **Useful counter reset** — periodically resets the "useful" counters to allow new entries to replace stale ones

Configurable parameters: `num_banks`, `table_size`, `loop_table_size`, `reset_interval`, `history_lengths`, `tag_widths`.

### SC-L-TAGE (Statistical Corrector + Loop + TAGE)

The most accurate predictor available. Combines four sub-predictors into a single high-accuracy predictor, following Seznec's Championship Branch Prediction (CBP) winning designs:

1. **TAGE** — same tagged geometric history as the standalone TAGE predictor (default: 8 banks)
2. **Loop Predictor** — detects counted loops and overrides TAGE when a loop iteration count is learned
3. **Statistical Corrector (SC)** — a bank of small signed counters indexed by different history lengths that learns to correct systematic TAGE errors. The SC sum is initialized with a centered confidence value from the TAGE prediction: `(2 * |ctr| + 1) * direction`. When the total SC sum disagrees with TAGE and exceeds a threshold, the SC prediction overrides TAGE.
4. **ITTAGE (Indirect Target TAGE)** — predicts indirect branch targets (computed jumps, virtual dispatch) using the same geometric history structure as TAGE but storing target addresses instead of direction counters

**USE_ALT_ON_NA** is also applied within SC-L-TAGE's TAGE component, ensuring the SC receives the effective TAGE prediction (after alt-pred override) rather than the raw provider prediction.

Configurable parameters: all TAGE parameters plus `sc_num_tables`, `sc_table_size`, `sc_history_lengths`, `sc_counter_bits`, `ittage_num_banks`, `ittage_table_size`, `ittage_history_lengths`, `ittage_tag_widths`, `ittage_reset_interval`.

## Predictor Comparison

Here's a representative comparison on the included benchmarks (width=1, default caches):

| Predictor | Accuracy (aggregate) | IPC (aggregate) | Speedup vs Static |
|-----------|---------------------|-----------------|-------------------|
| Static | 34.4% | 0.49 | 1.00× |
| GShare | 60.6% | 0.55 | 1.08× |
| Perceptron | 67.9% | 0.58 | 1.11× |
| Tournament | 70.9% | 0.59 | 1.20× |
| TAGE | 73.2% | 0.58 | 1.21× |
| SC-L-TAGE | 84.1% | 0.66 | 1.29× |

SC-L-TAGE provides the highest accuracy by combining TAGE with statistical correction and loop prediction. On `qsort`, SC-L-TAGE achieves 82.5% accuracy and 0.67 IPC versus standalone TAGE's 71.2% and 0.58 IPC — a 15.8% IPC improvement. Run `scripts/analysis/branch_predict.py` to regenerate numbers for your workloads.
