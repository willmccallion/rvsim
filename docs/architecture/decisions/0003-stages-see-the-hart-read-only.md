# 3. Every stage but commit sees the hart read-only

**Context.** All pipeline stages received one context with mutable access to
the hart, so any stage could change architectural state. Several did: the
in-order backend wrote vector results into the architectural registers at
execute, and fetch kept its PC on the hart.

**Decision.** Stages from fetch through writeback receive a `StageCtx`
(`sim/state/views.rs`). It reads the hart, drives the core's units (TLBs,
predictor, caches), and exposes only `counter()` and `events()` of the
uncore. Commit, traps and the engine's redirects use `CoreCtx`, which can
write the hart. Before the split, the offending writes were moved: vector
results are staged in a `ShadowVpr` and landed at commit, and the fetch PC
lives in the frontend while `Hart.pc` is the architectural PC.

**Consequences.** A register or CSR write from a stage does not compile;
compile-fail doctests on `StageCtx` prove it. CSR reads and translation are
free functions both views share.
