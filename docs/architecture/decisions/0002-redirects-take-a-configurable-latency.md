# 2. Execute-stage redirects take a configurable latency

**Context.** A branch misprediction or other execute-time redirect used to
change the fetch PC in the cycle it executed, and fetch ran at the new PC in
the same tick. gem5's O3 takes `iewToCommitDelay + commitToFetchDelay`
(2 cycles) to get a squash from execute to fetch; MinorCPU takes 1.

**Decision.** Execute returns a `Redirect` (`core/pipeline/squash.rs`). The
engine files a `PendingSquash` for it at the result's completion cycle, and
takes it `pipeline.redirect_latency` cycles later: default 2 for O3 and 1 for
in-order. Until then commit retires nothing the squash will remove. O3 keeps
issuing wrong-path work meanwhile, as gem5's IEW does; in-order issues
nothing the squash will remove, as Minor drops those instructions at once.
Predictor repair happens when the squash is taken. Memory-order and
coherence violations squash through the same path.

**Consequences.** The misprediction penalty includes the time the squash
takes to reach fetch. `tests/unit/core/pipeline/timing.rs` pins the exact
redirect cost on both backends.
