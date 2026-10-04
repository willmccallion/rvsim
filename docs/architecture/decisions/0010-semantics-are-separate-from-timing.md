# 10. Instruction semantics are separate from timing

**Context.** The two backends each contained parts of execute: the
in-order backend executed vector instructions against the architectural
registers itself, decoding lived in the frontend, and the retire effects
lived in commit. Each backend's copy could drift from the other's, and a
semantic bug fixed in one stayed in the other.

**Decision.** The `exec` layer defines what every instruction does, once:
decoding into an `exec::Inst` with its control signals, executing it, the
values loads produce, `vsetvl`, the vector and floating-point units'
arithmetic, and the effects commit applies on retirement. Execute reads
the hart through the `ArchState` trait (the hart, CSR reads, and whether
to trace) rather than through a pipeline's view, so it depends on nothing
in `uarch`. The pipelines decide when an instruction may issue, which unit
holds it for how long, and when its result becomes visible; for what the
result is, they call `exec`.

**Consequences.** Both backends compute identical results by construction,
so a difference between them is a timing-model bug. The riscv-tests and
the spike vector cross-check exercise one implementation of each
instruction. `exec` sits below the simulation kernel in the layering and
can be tested without building a pipeline.
