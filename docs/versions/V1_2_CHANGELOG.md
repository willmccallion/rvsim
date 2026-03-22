# Changelog — v1.2.0

Released: 2026-03-21

Multi-cycle squash recovery modeling, new SC-L-TAGE + ITTAGE branch predictor, branch predictor code reorganization, and several pipeline accuracy fixes.

## Added

### Multi-Cycle Squash Recovery

Models the physical bandwidth limit of ROB entry reclamation during pipeline flushes. The processor can only process `width` ROB entries per cycle during squash, so recovery is no longer instantaneous.

- **Squash walk penalty**: `ceil(squashed_entries / width) - 1` additional stall cycles after any flush (branch misprediction, trap, memory violation). Dispatch is blocked while the walk is in progress; all other stages continue draining normally.
- **Rename map rebuild penalty**: when no checkpoint is available (memory violations, CSR/FENCE flushes), an additional `ceil(surviving_entries / width)` cycles are added for forward-walking surviving ROB entries to reconstruct the speculative rename map.

### Checkpoint Table

O(1) branch misprediction recovery via rename map snapshots, eliminating the rename rebuild penalty for branch/jump flushes.

- **Rename-stage allocation**: a checkpoint is allocated for every branch/jump at dispatch time, capturing the speculative rename map after the instruction's own destination rename.
- **Stall on full**: when all checkpoint slots are occupied, rename stalls until a slot is freed by commit.
- **Commit-time release**: checkpoints are freed as their owning branch/jump commits or is flushed.
- Configurable via `checkpoint_count` (default 32). Set to 0 to disable checkpoints entirely (always rebuild from ROB).

### New Statistics

- `stalls.squash` — total dispatch stall cycles during squash recovery (ROB walk + rename rebuild).
- `stalls.checkpoint` — dispatch stall cycles due to a full checkpoint table.
- `stalls.rename_rebuild` — stall cycles specifically from rename map rebuild (no checkpoint available).

### SC-L-TAGE + ITTAGE Branch Predictor

New composed predictor following Seznec's CBP-winning designs, selected via `BranchPredictor.ScLTage(...)`.

- **Statistical Corrector (SC)**: correction layer that can flip the TAGE base prediction when confident it is wrong. Uses centered confidence per Seznec's design (`(2*|ctr|+1) * direction`) and 3-bit counters with adaptive threshold.
- **Loop Predictor**: detects counted loops and overrides TAGE with high-confidence loop iteration predictions. SC is bypassed when the loop predictor fires.
- **ITTAGE**: indirect target predictor using geometric-history tagged tables. Predicts targets for indirect branches (JALR non-return) where the BTB alone is insufficient.
- **USE_ALT_ON_NA**: meta-counter mechanism (added to both standalone TAGE and SC-L-TAGE) that prefers the alternate prediction when a provider entry is newly allocated with a weak counter.

### Branch Predictor Refactor

- Reorganized predictor code into `predictors/` and `components/` submodules.
- Extracted shared infrastructure (`GeoBankSet`, `FoldedHistory`) from the monolithic TAGE implementation into reusable components shared by TAGE, SC-L-TAGE, and ITTAGE.

### Python API

- `BranchPredictor.ScLTage(...)` configuration type with SC and ITTAGE sub-config parameters.

## Fixed

- **Decode RAW hazard checks with O3 backend**: decode previously broke superscalar bundles on intra-bundle RAW hazards even with register renaming enabled. Since rename resolves all RAW hazards by mapping source operands to physical registers before updating the rename map for the destination, these splits created unnecessary 1-cycle bubbles. Decode now skips this check when using the O3 backend.
- **CSR reads no longer flush the pipeline**: pure CSR reads (`CSRRS`/`CSRRC` with `rs1=x0`, `CSRRSI`/`CSRRCI` with `uimm=0`) previously triggered a full pipeline flush. They are still serializing at issue time but no longer flush younger instructions since they have no side effects.
- **FENCE no longer serializes in the issue queue**: FENCE previously waited for all older instructions to complete before issuing (full serialization). It now uses its own granular check that only waits for operations matching its predecessor bits, avoiding unnecessary pipeline drains.
