# 12. The model follows real cores; gem5 is the reference

**Context.** rvsim's timing was built by lining it up against gem5's O3
CPU, cycle by cycle, which found real bugs (a three-cycle-late recovery
from mispredictions, a missing ROB squash width). It also tempted the
model to copy behaviour that is particular to gem5 as a program: gem5
retires an instruction two cycles after writeback because its commit
stage marks completions after its retire pass, relays freed load-queue
entries to rename through IEW, and issues a store only when its address
and data are both ready. A model built to match those would be a gem5
clone rather than a model of a core.

**Decision.** rvsim models what a real core does. gem5 remains the
reference the timing is measured against, and its traces are how
differences are found, but when gem5's behaviour comes from its simulator
structure rather than from hardware, rvsim keeps the hardware behaviour
and the comparison records the difference in
`tools/gem5_compare/README.md` and the [error page](../../error.md).
Configuration parameters exist for things real cores differ on (forwarding
latency, squash width, queue sizes, widths), not to reproduce gem5. The
presets are calibrated against published measurements of the hardware
they name, and the [Linux benchmarks](../../examples/linux-benchmarks.md)
and the cache-latency probe check them.

**Consequences.** The error against gem5 is not a target to drive to zero:
some of it is gem5's. Each remaining difference is explained on the error
page as either a known rvsim gap (an issue) or a deliberate departure.
Hardware figures are the final check, where they exist.
