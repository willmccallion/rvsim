# Design

How rvsim is put together and why. The other architecture pages describe
what each part models; this one describes the structure that holds them,
the choices behind it, and the rules that keep it that way. The numbered
[design decisions](decisions/index.md) record each choice in more detail.

## Goals

- **Model real cores.** A configuration describes a machine, and the
  simulator should run it the way that machine runs: pipeline depth, queue
  sizes, latencies and the ordering rules of the memory system. gem5's O3
  CPU is the reference rvsim measures itself against cycle by cycle, but
  where gem5 behaves the way a simulator does rather than the way a core
  does, rvsim follows the core and the comparison records the difference
  ([decision 12](decisions/0012-the-model-follows-real-cores.md)).
- **Determinism.** The same program and configuration give the same cycle
  count and statistics on every run and every host.
- **One definition of each behaviour.** What an instruction does is
  written once and shared by both pipelines and by checkpointing; the
  pipelines differ only in when and how fast it happens.
- **Structure the compiler enforces.** Who may change architectural
  state, which core may touch which state, and which modules may depend on
  which are checked by the type system and the module visibility, not by
  convention.

## Layers

`rvsim-core` is nine modules, each depending only on those before it:

| # | Module | Holds |
|---|---|---|
| 1 | `common` | Addresses, identifiers, access kinds, tracing |
| 2 | `isa` | What the ISA defines: encodings, CSRs, register names, operation vocabulary |
| 3 | `config` | Simulator configuration, parsed and validated |
| 4 | `arch` | Architectural state: harts, register files, CSRs, traps, PMP, translation |
| 5 | `exec` | Instruction semantics every engine shares: decode, execute, retire effects |
| 6 | `sim` | The simulation kernel: event queue, packets, component handles, the memory image, the statistics tree |
| 7 | `soc` | The uncore: bus, caches, coherence fabric, memory controllers, devices |
| 8 | `uarch` | A core's timing model: frontend, the two backends, MMU, branch predictors, the views a pipeline runs on |
| 9 | `system` | The whole system: `Simulator`, its state, loading, checkpoints, device tree |

Layers 5 to 8 are the model and are crate-private; the first four and the
last are the published interface
([decision 8](decisions/0008-the-model-is-crate-private.md)). A user of the
crate builds a `Simulator` from a `Config`, runs it, and reads harts,
memory, statistics and a plain-data pipeline snapshot back; the internals
can change without a breaking release.

**Why layers.** Before the September 2026 restructuring the crate was a
`core` module that held everything from instruction encodings to the
pipeline, and a `sim` module that mixed the event kernel with the system
that used it. Encodings depended on pipeline types, execute semantics
lived inside pipeline stages, and the stats tree knew every component's
counter paths. Layering made each dependency an explicit choice: the
kernel (`sim`) knows nothing about caches or cores (components register
their own statistics through `StatSource`), and the ISA and architectural
state know nothing about timing.

## Who owns state

```
SystemState
├── harts:  Vec<Hart>     architectural state, one per hardware thread
├── cores:  Vec<Core>     one per core
│   ├── units: CoreUnits  L1I, L1D, L2, MMU, write-combining buffer, branch predictor
│   └── pipeline          the in-order or out-of-order pipeline
└── uncore: Uncore        shared: topology, clock, event queue, bus, LLC,
                          coherence fabric, memory controller, memory image,
                          configuration, statistics
```

**The split.** The simulator used to keep one `SimState` that held a hart,
a core, the caches, the bus, the memory controller, the event queue, the
statistics and the configuration, and every pipeline stage received a
mutable reference to all of it. That made it a god object: any stage could
change any state, which is how execute-stage writes to architectural
registers kept appearing, and adding a second core meant every function
that said "the hart" was ambiguous. It was split into arenas indexed by
identifier, each owned once
([decision 9](decisions/0009-state-is-split-into-harts-cores-and-an-uncore.md)):

- **`Hart`** is architectural state only: registers, CSRs, privilege, PC.
  It is what a checkpoint saves and what a test inspects.
- **`Core`** is one core's private hardware and its pipeline
  ([decision 7](decisions/0007-a-core-owns-its-pipeline.md)).
- **`Uncore`** is everything the cores share.

**Views instead of the whole.** A pipeline never sees `SystemState`. It
works on a view built from disjoint borrows of one hart, one core and the
uncore, so a core cannot reach another core's state:

- **`CoreCtx`** has all three mutable. Commit holds it, and so do traps
  and redirects, the only places architectural state legitimately
  changes.
- **`StageCtx`** is every other stage's view: the hart read-only, the
  core's micro-architecture mutable, and only the statistics and event
  queue of the uncore. A register or CSR write from a stage does not
  compile; compile-fail doctests pin that down
  ([decision 3](decisions/0003-stages-see-the-hart-read-only.md)).

`CoreCtx::stage()` narrows the one into the other. Translation, CSR reads
and trigger checks are free functions both views share, so narrowing
loses no capability a stage should have.

**Identifiers from one place.** `Topology` assigns every `CoreId`,
`HartId`, `PipelineId`, `CacheId` and `MemCtrlId` from the configuration.
Routing asks it where an identifier lives instead of relying on numbering
conventions, and request identifiers carry their pipeline in their top
bits, so shared caches and memory controllers never confuse two cores'
requests.

## Semantics and timing

`exec` defines what every instruction does, once. Decoding produces an
`exec::Inst` with its control signals; executing it reads the hart through
the `ArchState` trait and returns a result, a trap or a redirect; retiring
it applies the effects commit makes architectural. Neither backend contains
instruction semantics of its own: the out-of-order and in-order pipelines
decide when an instruction may issue, which unit it occupies for how long,
and when its result is visible, and call the same functions for what the
result is.

**Why.** The in-order and out-of-order backends used to carry their own
copies of parts of execute, and they drifted: the in-order backend wrote
vector results into architectural registers at execute. With one
definition, a semantic bug is fixed once, the riscv-tests and the spike
vector cross-check exercise both backends equally, and any difference in
results between the backends is a timing-model bug by construction.

## The simulation kernel

Components that do not belong to a pipeline (caches, the bus, the
coherence fabric, memory controllers, devices) are event-driven. Each
implements `Handle`: it receives a typed `Packet` from a source
`ComponentId` and may schedule further packets. All of them share one
`EventQueue`, a min-heap ordered by fire cycle and then by a sequence
number assigned at scheduling, so ties resolve in a fixed order and runs
are deterministic.

Pipelines are not event-driven. The simulator ticks every core's pipeline
once per cycle, in core order, and pipelines talk to the memory system
only through packets: a load becomes a request to the L1D and the response
lands in the pipeline's mailbox. This keeps pipeline code a straight
sequence of stages, which is easy to read and to line up against a
reference trace, while the components between them keep explicit
latencies and occupancy.

**Idle time is skipped, not ticked.** When every core waits in `WFI` with
nothing in flight and no device, timer or memory controller is due, the
simulator advances the clock to the next event in one step. The skip
updates every counter a tick would, and a test checks that a run with
skipping enabled is identical, cycle for cycle and stat for stat, to one
without it.

## Memory: one image, accesses take effect where they are served

Caches hold tags and state only. There is one memory image
(`sim::memory::GlobalMemory`), which owns RAM, the LR/SC reservations and a
log of RAM writes, and every `Handle` receives it. A load reads it and a
store writes it at the moment the component that serves the access does
so: an L1D hit as it arrives, a miss when its fill returns, an access no
cache serves at the memory controller. Each write records who made it (a
hart's store, a device's DMA, the host).

**Why.** Keeping data in one place means a coherence or timing bug cannot
make two copies of a line disagree; it can only make an access happen at
the wrong time, which is the kind of error a timing simulator should be
able to show. The perform point gives multi-core runs real memory-model
behaviour: a load that read a line before another hart's store reached it
sees the old value, and the write log lets the load queue detect when a
younger load read too early. Owning RAM in one value also removed the
shared raw pointers that the controllers, the bus and the virtio device
used to hold, and with them the `unsafe impl Send + Sync` that papered over
them ([decision 11](decisions/0011-accesses-take-effect-where-they-are-served.md)).

## Pipelines

Both backends sit behind the `ExecutionEngine` trait. They share the
frontend (fetch, decode, and the rename stage that hands instructions to
the engine) and the memory and commit stages (memory1, memory2,
writeback, commit), which live in `backend::shared`, along with the
bookkeeping every engine needs for requests in flight (`BackendCommon`:
the mailbox, outstanding loads, stores and walks, the pending squash).
They differ in how an instruction gets from rename to execute: the
out-of-order engine renames onto physical registers and issues from a
CAM-style queue with wakeup and select; the in-order engine tracks
operands with a scoreboard and issues in program order, as many per cycle
as its width and its functional units allow.

The pipeline's structure follows a few rules, each recorded as a decision:

- **Latches carry an explicit delay.** A bundle written to a latch in one
  cycle is visible a stated number of cycles later, independent of the
  order stages run in within a tick
  ([decision 4](decisions/0004-latches-carry-an-explicit-delay.md)).
- **Redirects take a latency.** A misprediction or other execute-time
  redirect reaches fetch `redirect_latency` cycles after it resolves, and
  commit retires nothing the pending squash will remove
  ([decision 2](decisions/0002-redirects-take-a-configurable-latency.md)).
- **Faults travel with the instruction.** A fault found at fetch, decode
  or execute is carried to commit and taken there, once
  ([decision 5](decisions/0005-faults-are-taken-at-commit.md)).
- **Serialisation follows the CPU model.** The out-of-order backend holds
  rename behind a serialising instruction until the ROB drains; the
  in-order backend squashes and refetches after it
  ([decision 6](decisions/0006-serialization-follows-the-cpu-model.md)).
- **Stores issue in two halves.** A plain store's address issues as soon
  as its base register is ready and its data when the value is, as real
  out-of-order cores split stores
  ([decision 13](decisions/0013-stores-issue-address-and-data-separately.md)).

## Configuration at the edge

Configuration is parsed once, where it enters, into types that carry their
guarantees, and checked as a whole before anything is built. A `Vlen` is
a power of two from 128 to 2048; a key the core does not know is refused
rather than ignored; and `Config::validate` rejects combinations no
machine can have: a BTB whose set count is not a power of two, a cache
line smaller than the 64-byte block cache-block operations act on, an
exclusive L1/L2 with more than one core, a vector extension without a full
vector configuration, TAGE banking that does not fit its tables. Everything
downstream takes the parsed types and does not re-check them. The Python `Config` is a flat, builder-style
description serialised to the same structure, and `rvsim/_core.pyi` is
checked against the compiled extension by the test suite, so the Python
signatures cannot drift from the Rust ones.

## Statistics

Statistics form one tree keyed by path (`core0.cache.l1d.misses`,
`hart1.retired_insts`, `system.traps`), with counters, histograms and
stats derived from them. Components register their own paths, so adding a
component adds its statistics without touching the kernel. Regions of
interest are measured by differencing snapshots, so measuring a region
leaves the whole-run statistics intact. See
[Stats & Observability](stats.md).

## Sessions and checkpoints

A checkpoint is the committed state: saving drains every pipeline first,
then records every hart, the memory image and the cycle counter.
`Session` builds on that to run a workload in phases, typically a fast
boot, a switch to the configuration under study, warm-up, and measured
regions. Fast-forwards are cached under a key made of the workload's
files, everything the session did before, the configuration and the stop,
so the second run of an experiment restores the boot instead of repeating
it.

## Keeping it this way

- **Refactors are cycle-identical.** A structural change must leave every
  cycle count unchanged, checked against a recorded baseline over the
  reference programs and a Linux boot, so a refactoring mistake cannot hide
  inside a timing change ([decision 1](decisions/0001-refactors-are-cycle-identical.md)).
- **Lints.** `clippy::pedantic` and `clippy::nursery` are denied across the
  workspace; there is no `unwrap`, `expect` or `panic!` on a production
  path, and every `unsafe` block names the invariant it relies on in a
  `// SAFETY:` line, with one unsafe operation per block.
- **Tests live with the model.** The Rust suite is inside the crate, so it
  can test crate-private units directly, and test-only probes are
  `#[cfg(test)]`. Timing tests assert exact cycle differences, so a change
  that shifts a latency by one cycle fails a test that names it.
- **Validation against two references.** Conformance (riscv-tests, the
  chipsalliance vector suite cross-checked against spike, multi-core
  litmus programs, a Linux boot) shows the model is functionally right.
  Timing is measured against gem5 ([Error against gem5](../error.md)) and
  against hardware ([Linux Benchmarks](../examples/linux-benchmarks.md)).
