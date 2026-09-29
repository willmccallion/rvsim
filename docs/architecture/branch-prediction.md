# Branch Prediction

rvsim implements six pluggable branch predictors with shared infrastructure. The predictor is consulted during Fetch1 to steer the instruction stream speculatively.

## Prediction Unit

Each direction predictor sits in a prediction unit with the BTB and the RAS,
modelled on gem5's `BPredUnit`. Fetch numbers every instruction it forms
(gem5's `InstSeqNum`) and asks the unit to predict each control instruction.
The unit keeps one record per prediction, oldest first: what the direction
predictor read, the histories before it, and the stack operations it did.

- **Commit** reports the youngest instruction it retired, and the predictor
  trains on every record up to it with what that prediction read.
- **A misprediction** squashes the records younger than the branch and
  rewrites the branch's own history update with its real outcome.
- **Every other squash** (a CSR access, a memory-order or coherence replay,
  a trap) undoes the records younger than the instruction it keeps,
  youngest first, restoring the speculative histories and the RAS
  exactly. A squash of the whole window undoes all of them.

A commit notice reaches the predictor the cycle after commit sends it
(gem5's `commitToFetchDelay` of 1), and waits while a squash is pending:
that squash may correct a prediction commit has already passed, and the
correction must reach the predictor first, as gem5's fetch takes a squash
before a commit notice.

## Shared Infrastructure

All predictors share these components:

### Branch Target Buffer (BTB)

Set-associative cache (default: 4096 entries, 4-way) that maps branch PCs to their target addresses. Used for indirect jumps where the target isn't encoded in the instruction.

As in gem5, the BTB learns a target only when a taken branch's misprediction
is corrected: for direct control flow, and for an indirect jump that is not
a return when the predictor has no indirect target predictor of its own
(SC-L-TAGE's ITTAGE keeps those). A jump that executes on a path an older
branch squashes before its own correction arrives teaches it nothing.

### Return Address Stack (RAS)

Circular stack (default: 32 entries) for call/return prediction, after
gem5's `ReturnAddrStack`: a push past capacity overwrites the oldest entry.
Fetch pushes a call's return address and pops a return's target the moment
the instruction is fetched, so a return fetched right behind its call is
predicted. Each prediction records its push and pop, including the entry a
pop exposed, and a squash undoes them youngest first.

Per RISC-V spec Table 2.1, both **x1 (ra)** and **x5 (t0)** are recognized as link registers:

| Instruction | rd is link? | rs1 is link? | Action |
|-------------|-------------|--------------|--------|
| `jal rd, offset` | Yes | — | **Push** return address onto RAS |
| `jal rd, offset` | No | — | No RAS action (plain jump) |
| `jalr rd, rs1, offset` | No | Yes | **Pop** from RAS (return) |
| `jalr rd, rs1, offset` | Yes | Yes, rd ≠ rs1 | **Pop then push** (coroutine swap) |
| `jalr rd, rs1, offset` | Yes | Yes, rd = rs1 | **Push** (call through link register) |
| `jalr rd, rs1, offset` | Yes | No | **Push** (indirect call) |

Direct control flow does not need the BTB: fetch takes a `jal` target,
and a conditional branch's target on a BTB miss, from the instruction's
immediate, as a predecoded fetch line lets a real front end. Compressed
control flow (`c.j`, `c.jr`, `c.jalr`, `c.beqz`, `c.bnez`) is predicted
exactly like its 32-bit expansion.

### Global History Register (GHR)

Arbitrary-length bit vector recording the direction (taken/not-taken) of recent branches. The GHR is speculatively updated during Fetch1 and restored on a squash from the records of the squashed predictions.

Every control instruction shifts the GHR at fetch with its predicted direction, as gem5's predictors do: a jump, call or return counts as taken unless fetch had no target for it. A branch reached through a jump sees a different path from one reached without. The GHR length is unlimited — it grows to match the longest history needed by the selected predictor (e.g., TAGE's geometric history lengths can exceed 700 bits).

## Predictors

### Static

Always predicts not-taken. Useful as a baseline for measuring how much a predictor contributes.

### GShare

XOR of the branch PC and the global history register indexes into a table of 2-bit saturating counters. Simple and effective for workloads with strong global correlation.

### Tournament

gem5's `TournamentBP`, the Alpha 21264 predictor, with three components:

1. **Global predictor** — 2-bit counters indexed by global history alone
2. **Local predictor** — a local history table indexed by `pc >> 2`, feeding a table of 2-bit counters
3. **Choice predictor** — 2-bit counters, also indexed by global history, selecting the global prediction when above 1

Counters start at 0. Both histories are updated speculatively at prediction
and restored per squashed branch. At commit the choice counter moves toward
whichever component was right, when they disagreed, using the predictions
recorded at fetch. A jump shifts only the global history and trains its
global counter as taken.

Configurable parameters: `global_size_bits` (global and choice tables), `local_hist_bits` (local history table), `local_pred_bits` (local counters).

### Perceptron

Neural branch predictor. Each entry in the table is a vector of integer weights, one per GHR bit. The dot product of the weight vector and the recent branch history determines the prediction. Weights are trained on mispredictions using a threshold-based update rule.

Configurable parameters: `history_length`, `table_bits`.

### TAGE (Tagged Geometric History Length)

Uses multiple tagged tables with geometrically increasing history lengths:

- **Base predictor** — TAGEBase's bimodal: a prediction bit per entry and a hysteresis bit shared by `2^bimodal_hysteresis_share_log` entries (default 4), `bimodal_entries` of them (default `table_size`; SC-L-TAGE defaults to 8192)
- **Tagged tables** — each table uses a different history length (default: 5, 11, 22, 44, 89, 178, 356, 712 for 8 banks). Entries are tagged with a hash of the PC and history to avoid aliasing.
- **Longest match wins** — the prediction comes from the table with the longest matching history
- **USE_ALT_ON_NA** — meta-counter that learns whether newly allocated (weak) provider entries should be trusted or whether the alternate (second-longest match) prediction is better. When the provider entry's counter is weak (0 or -1) and the meta-counter is non-negative, the alternate prediction is used instead.
- **Useful counter reset** — periodically resets the "useful" counters to allow new entries to replace stale ones

Configurable parameters: `num_banks`, `table_size`, `reset_interval`, `history_lengths`, `tag_widths`, `use_alt_counters` and `use_alt_bits` (one 4-bit USE_ALT_ON_NA counter, as TAGEBase keeps; SC-L-TAGE defaults to TAGE-SC-L's 16 5-bit counters, indexed by the provider's bank group and the alternate's confidence), `useful_bits`, `max_allocations`, and `allocation` / `update` (`"tage_base"`, TAGEBase's rules with useful bits halving every `reset_interval` updates, or `"cbp5"`, the CBP-5 TAGE-SC-L rules: pairwise allocation that decays strong entries in the way, one allocation in 32 when the final prediction was right, useful bits halving once allocation penalties reach `reset_interval`; SC-L-TAGE defaults to `cbp5` with 1-bit useful counters, two allocations and an interval of 1024), `history` (`"direction"`, TAGEBase's one direction bit per control instruction, or `"pc_bits"`, TAGE-SC-L's two PC-hashed bits per instruction and three per indirect jump; SC-L-TAGE defaults to `pc_bits` over lengths 6 to 3000) and `path_history_bits` (16, or TAGE-SC-L's 27). `hashing` (`"tage_base"`, the PC shifted past its low bits, or `"tage_sc_l"`, the whole PC and a `pc ^ (pc >> 2)` bimodal index) and `banking` (`BranchPredictor.TageBanking`: TAGE-SC-L's pairs of banks forming 2-way tables, carved from a short-tag and a long-tag array in PC- and path-hashed slices, with an `enabled` flag per bank). SC-L-TAGE defaults to the 64KB TAGE-SC-L's TAGE: 36 banks of 1024 entries, 8-bit tags for the first 12 and 12-bit after, arrays of 10 and 20 slices, gem5's enabled banks, and 18 history lengths from 6 to 3000 shared in pairs. The global history is a circular buffer; each prediction keeps a checkpoint of its head, path and folded histories, which a squash restores without recomputing the folds.

### SC-L-TAGE (Statistical Corrector + Loop + TAGE)

The most accurate predictor available. Combines four sub-predictors into a single high-accuracy predictor, following Seznec's Championship Branch Prediction (CBP) winning designs:

1. **TAGE** — same tagged geometric history as the standalone TAGE predictor (default: 8 banks)
2. **Loop Predictor** — Seznec's loop predictor: a set-associative table that learns a loop's trip count, tracks each loop's iteration speculatively (restored on a squash), and overrides TAGE only when an entry is confident and a use counter shows loop overrides have been helping
3. **Statistical Corrector (SC)** — Seznec's corrector: three bias tables (indexed by the PC, the prediction before the corrector and TAGE's confidence) and GEHL components over backward-branch, path, three local, and two IMLI (inner-most loop iteration) histories. Each component's sum of centred counters is doubled or not by a learnt per-PC weight. When the total disagrees with the prediction before it, the corrector wins unless TAGE is confident and the total is small, where two chooser counters decide. Its histories advance speculatively and are restored on a squash; each prediction carries what it read to commit, so it trains the counters it voted with.
4. **ITTAGE (Indirect Target TAGE)** — predicts indirect jump targets (computed jumps, virtual dispatch) using the same geometric history structure as TAGE but storing target addresses instead of direction counters; it trains on each committed indirect jump's real target

**USE_ALT_ON_NA** is also applied within SC-L-TAGE's TAGE component, ensuring the SC receives the effective TAGE prediction (after alt-pred override) rather than the raw provider prediction.

Configurable parameters: all TAGE parameters plus the `loop_*` loop predictor parameters (`loop_log_size`, `loop_log_assoc`, `loop_tag_bits`, `loop_iter_bits`, `loop_confidence_bits`, `loop_age_bits`, `loop_use_counter_bits`, `loop_use_direction_bit`, `loop_use_hashing`, `loop_restrict_allocation`, `loop_initial_iter`, `loop_initial_age`, `loop_optional_age_reset`, `loop_long_loop_confidence`, `loop_optional_age_increment`), the `sc_*` corrector parameters (`sc_log_bias`, `sc_counter_bits`, `sc_weight_bits`, `sc_bias_weight_init`, `sc_chooser_bits`, `sc_threshold_bits`, `sc_initial_threshold`, `sc_per_pc_threshold_bits`, `sc_per_pc_threshold_width`, `sc_initial_per_pc_threshold`, `sc_threshold_weight_step`, `sc_halve_short_tables`, `sc_imli_counter_bits`, and the components `sc_global`, `sc_backward`, `sc_path`, `sc_imli`, `sc_imli_history` as `BranchPredictor.ScGehl` and `sc_local` as a list of `BranchPredictor.ScLocalGehl`), `ittage_num_banks`, `ittage_table_size`, `ittage_history_lengths`, `ittage_tag_widths`, `ittage_reset_interval`.

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

SC-L-TAGE provides the highest accuracy by combining TAGE with statistical correction and loop prediction. On `qsort`, SC-L-TAGE achieves 82.5% accuracy and 0.67 IPC versus standalone TAGE's 71.2% and 0.58 IPC — a 15.8% IPC improvement. Run `examples/analysis/branch_predict.py` to regenerate numbers for your workloads.
