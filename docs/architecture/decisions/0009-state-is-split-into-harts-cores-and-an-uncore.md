# 9. System state is split into harts, cores and an uncore

**Context.** The simulator kept one `SimState` holding a hart, a core, the
LLC, the bus, the memory controller, the event queue, the statistics, the
configuration, the instruction count and the exit state. Every pipeline
stage received it mutably. Any stage could change any state, and did:
execute-stage writes to architectural registers kept appearing, and the
instruction count lived beside the caches rather than on the hart. It also
assumed one hart: every function that said "the hart" or "the core" would
have become ambiguous with a second.

**Decision.** `SystemState` owns three things, each once:

- `harts: Vec<Hart>`, architectural state only (registers, CSRs,
  privilege, PC, retired-instruction count), indexed by `HartId`;
- `cores: Vec<Core>`, one core's private hardware and its pipeline,
  indexed by `CoreId` ([decision 7](0007-a-core-owns-its-pipeline.md));
- `uncore: Uncore`, everything the cores share: topology, clock, event
  queue, bus, LLC, coherence fabric, memory controller, the memory image,
  configuration and statistics.

A pipeline never sees `SystemState`. `SystemState::core_ctx` builds a
`CoreCtx` from disjoint borrows of one hart, one core's units and the
uncore. `CoreCtx` derefs to the uncore so uncore fields read as before, and
every former single-hart operation (translation, traps, CSR access,
triggers, reservations) became a method on it. `Topology` assigns every
component identifier from the configuration, and request identifiers carry
their pipeline in their top bits so shared components cannot confuse two
cores' requests.

**Consequences.** A core cannot reach another core's state; the borrow
checker refuses it. Adding cores is a configuration change. The split also
made [decision 3](0003-stages-see-the-hart-read-only.md) possible: once a
pipeline worked on a view rather than the whole, the view handed to
non-commit stages could leave the hart read-only. Single-core behaviour
was cycle-identical across the change
([decision 1](0001-refactors-are-cycle-identical.md)).
