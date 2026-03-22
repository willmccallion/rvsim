# Changelog — v1.1.0

Released: 2026-03-21

Memory dependence prediction support for the out-of-order backend, with two configurable predictor backends and associated bug fixes in the memory pipeline.

## Added

### Memory Dependence Unit (MDP)

Pluggable predictor framework with two backends:

- **Blind**: conservative predictor where loads always wait for all older stores to resolve addresses before issuing. No speculation, no violations. This is the default.
- **StoreSet**: speculative predictor based on Chrysos & Emer (ISCA 1998). Learns load-store dependencies from observed ordering violations and allows loads predicted as independent to bypass unresolved stores.

### Store Set Predictor Internals

- **Store Set Identifier Table (SSIT)**: maps load and store PCs to store set IDs via `(pc >> 2) % ssit_size`. Periodically cleared to prevent stale dependencies from persisting.
- **Last Fetched Store Table (LFST)**: tracks the most recent in-flight store per store set ID, used to establish dispatch-time dependencies between loads and stores.
- **Dispatch-time dependency caching**: issue queue entries record their MDP prediction state (Bypass, WaitFor, or WaitAll) at dispatch so the predictor is not re-queried during wakeup.
- **Explicit wakeup path**: when a store resolves its address, the issue queue wakes any entries in WaitFor state that were waiting on that store's ROB tag.
- **Partial flush with LFST rebuild**: on pipeline squash, the LFST is rebuilt from surviving in-flight stores in the ROB rather than being fully cleared.

### Configuration

- `StoreSetConfig` with configurable SSIT size (default 2048), LFST size (default 256), and SSIT clear interval (default 100,000 cycles).

### Python API

- `MemDepPredictor.Blind()` and `MemDepPredictor.StoreSet(ssit_size, lfst_size)` configuration types, passed via `Config(mem_dep_predictor=...)`.

### Statistics

Four new counters exposed through both Rust stats and Python bindings:

- `mdp_predictions_bypass` — loads predicted to bypass all older stores
- `mdp_predictions_wait_all` — loads predicted to wait for all older stores
- `mdp_predictions_wait_for` — loads predicted to wait for a specific older store
- `mdp_violations` — ordering violations reported to the predictor

Printed under a "Memory Dependence Prediction" section in stats output when non-zero.

## Fixed

- **SC and AMO ordering violation checks**: `memory2_stage` now checks for load ordering violations when SC (store-conditional) and AMO (atomic read-modify-write) operations resolve their addresses. Previously, only regular stores triggered these checks, meaning a younger load that speculatively read an address later written by an SC or AMO would not be detected as a violation.
- **Violation loss on memory2 stall**: when `memory2_stage` stalled (e.g., due to an older store blocking a load or AMO), any ordering violation already detected earlier in the same cycle was discarded. The stage now preserves and returns accumulated violations through all early-return paths.
