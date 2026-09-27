# 5. Faults travel with the instruction and are taken at commit

**Context.** An instruction that faulted at execute marked its ROB entry and
squashed everything younger from execute; commit then flushed again when it
took the trap. On O3 a trap-carrying result from a non-memory unit completed
its ROB entry without faulting it, so debug-trigger breakpoints were lost.

**Decision.** A fault raised at execute (illegal instruction, ECALL, a debug
trigger, or a trap carried from fetch or decode) travels on the execute
result. Writeback records it on the ROB entry, and commit takes it and
flushes everything younger once, as a real core and gem5 do. The same holds
for both backends.

**Consequences.** Instructions younger than a fault may execute before it
is taken; they never retire. A fault costs one flush instead of two.
Regression tests: `tests/integration/execute_trigger.rs`,
`fault_precedence.rs`, `trap_latency.rs`.
