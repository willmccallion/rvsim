# 4. Frontend latches carry an explicit delay

**Context.** Stage-to-stage latches were plain vectors, so the cycles a
bundle took between stages depended on the order stages ran in within a
tick rather than on a stated delay.

**Decision.** The frontend latches and the rename-to-dispatch latch are
`Latch<T>` (`core/pipeline/latches.rs`): one bundle, readable `delay` cycles
after it is written, and the producer runs only when the consumer has
emptied it. The delays match the previous behaviour (fetch1→fetch2 0 because
the I-cache response already carries the fetch latency; the others 1).

**Consequences.** Each delay is visible and testable. The latches hold a
single bundle; gem5's multi-bundle fetch queue and configurable stage
delays are not modelled yet.

**Amended (#213).** The front end's depth is a configuration of the core,
not a constant: `fetch_decode_latency`, `decode_rename_latency` and
`rename_issue_latency` set the three latches' delays, and a latch with no
delay hands its bundle on within the cycle by running its consumer again
after its producer. The defaults are the depth above; the `rocket` preset
collapses decode and rename into Rocket's ID stage and issues the cycle
after it, the `boom` preset adds BOOM's F3.
