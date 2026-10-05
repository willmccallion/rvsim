# Design decisions

Each record states a decision about the simulator's structure or timing
model, why it was taken, and what follows from it. Records are not edited
after the fact; a later decision that changes one says which it replaces.

| # | Decision |
|---|----------|
| [1](0001-refactors-are-cycle-identical.md) | Structural changes are cycle-identical and gated |
| [2](0002-redirects-take-a-configurable-latency.md) | Execute-stage redirects take a configurable latency |
| [3](0003-stages-see-the-hart-read-only.md) | Every stage but commit sees the hart read-only |
| [4](0004-latches-carry-an-explicit-delay.md) | Frontend latches carry an explicit delay |
| [5](0005-faults-are-taken-at-commit.md) | Faults travel with the instruction and are taken at commit |
| [6](0006-serialization-follows-the-cpu-model.md) | Serialization follows the gem5 CPU model each backend stands for |
| [7](0007-a-core-owns-its-pipeline.md) | A core owns its pipeline |
| [8](0008-the-model-is-crate-private.md) | The model is crate-private |
| [9](0009-state-is-split-into-harts-cores-and-an-uncore.md) | System state is split into harts, cores and an uncore |
| [10](0010-semantics-are-separate-from-timing.md) | Instruction semantics are separate from timing |
| [11](0011-accesses-take-effect-where-they-are-served.md) | Accesses take effect where they are served |
| [12](0012-the-model-follows-real-cores.md) | The model follows real cores; gem5 is the reference |
| [13](0013-stores-issue-address-and-data-separately.md) | Stores issue their address and data separately |
| [14](0014-prefetchers-follow-published-hardware.md) | Prefetchers follow the published hardware |
