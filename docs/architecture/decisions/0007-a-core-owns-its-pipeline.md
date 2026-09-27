# 7. A core owns its pipeline

**Context.** The simulator held `pipelines: Vec<PipelineDispatch>` beside
`state.cores: Vec<Core>`, two vectors related only by index.

**Decision.** `Core` is the per-core model: `units: CoreUnits` (caches,
MMU, write-combining buffer, branch predictor) and `pipeline:
PipelineDispatch`. `SimState::pipeline_ctx` hands out the pipeline together
with a `CoreCtx` built from disjoint borrows of the hart, the units and the
uncore. Pipelines are built with the state and pointed at the loaded PC when
the `Simulator` is created.

**Consequences.** There is one per-core collection, indexed by `CoreId`.
The context types refer to `CoreUnits`, so a pipeline cannot reach another
pipeline, or its own, through its context.
