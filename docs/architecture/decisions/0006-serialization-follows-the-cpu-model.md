# 6. Serialization follows the gem5 CPU model each backend stands for

**Context.** gem5 marks CSR accesses, ECALL, xRET, WFI, SFENCE.VMA and
FENCE.I `IsSerializeAfter`. Its two CPU models treat that flag differently,
and so do real in-order and out-of-order cores.

**Decision.**

- **In-order**, like MinorCPU, squashes and refetches after every
  serializing instruction, CSR reads included.
- **O3**, like gem5's O3 rename, holds the instruction after a serializing
  one in rename until the ROB has drained, learning of the drain a cycle
  after commit empties it (`o3/serialize.rs`, counted in
  `pipeline.stalls.serialize`). A CSR write no longer squashes.
- Both backends issue system instructions only from the ROB head, and hold
  younger loads behind an uncommitted CBO, which takes effect at commit.

**Consequences.** The two backends differ here on purpose. gem5's O3 also
serializes after store-conditionals; that is a gem5 simplification and is
not modelled. CBOs block every younger load rather than only those to the
same block, which is conservative.

**Amended (#212).** The gem5 behaviour is the default, not the model: a
preset of a real core sets `csr_squash` and `fence_squash` from the core's
decoder. Rocket flushes after a write to any CSR but the scratch, epc,
cause and tval ones and never after a read (`write_flush` in
`rocket/CSR.scala`); BOOM flushes when any CSR instruction or FENCE commits
(`flush_on_commit` in `v4/exu/decode.scala`). The in-order backend takes
the flush when the access executes, the out-of-order one when it commits,
on top of its rename hold.
