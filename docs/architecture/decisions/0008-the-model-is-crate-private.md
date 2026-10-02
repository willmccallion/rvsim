# 8. The model is crate-private

**Context.** `rvsim-core` is published, and every one of its modules was
`pub`: the event kernel, the caches, the DDR5 controller, the pipeline
stages, the branch predictors. Two things kept them that way. The Rust
test suite lived in an external test crate and could only reach `pub`
items, and the Python bindings built the system by hand from `SystemState`
and the uncore's structures. The result was a semver surface of several
thousand items, most of them timing-model internals that change with
every accuracy fix, and `dead_code` could not see anything the model had
stopped using, because a `pub` item is never dead.

**Decision.** The crate's interface is the system. `common`, `isa`,
`config`, `arch` and `system` are `pub`; `exec`, `sim`, `soc` and `uarch`
are `pub(crate)`. `Simulator` offers what a host needs: loading a program
or kernel, running, reaching a hart's registers, CSRs and translations,
the statistics tree (re-exported as `rvsim_core::stats`), the trace and
console channels, and a plain-data `system::snapshot::PipelineSnapshot` of
the latches. The test suite lives inside the crate under `src/tests`, and
an accessor that only a test reads is `#[cfg(test)]`.

**Consequences.** The published surface is what the bindings use and
nothing else, so a timing-model change is not a breaking change. Making
the model private let the compiler find what the model no longer used:
packet variants never sent (`Fence`, `Prefetch`, `RefreshTick`), a MOESI
state the MESI protocol never entered, reorder-buffer fields written and
never read, a legacy tag-wakeup path in the issue queue, and some sixty
accessors and helpers; they are gone. Thirty-one accessors that tests use
as probes are `#[cfg(test)]`, which states their only consumer.

The compile-fail doctests that [decision 3](0003-stages-see-the-hart-read-only.md)
cites went with `StageCtx`'s public path; a doctest cannot name a private
type. The guarantee is unchanged and rests on the type: `StageCtx::hart`
returns a shared reference, and the view has no CSR or memory write.
