# Multi-core architecture

This document is the design for multi-core rvsim: how the system is
decomposed, how cores share the uncore, how coherence is modelled, and the
invariants every future extension must keep. It is written before the code
so that the structure does not need to change as cores, cache levels,
coherence protocols, interconnects and devices are added.

## Goals

1. **Determinism.** Same binary, config and seed produce the same cycle
   count and the same stats, run after run, on any machine. No wall-clock
   input, no unordered iteration on any timing path, one global event
   queue with a sequence-number tiebreak.
2. **Cycle accuracy at every level.** Cores, caches, interconnect and memory
   controllers are each components with their own state and timing; no
   component reaches into another's state. Clock domains are explicit.
3. **Modularity without inheritance.** Every strategy-shaped decision
   (replacement, prefetch, scheduling, refresh, coherence protocol,
   interconnect topology, interrupt routing) is a trait with several
   implementations chosen by config. Components communicate only through
   typed packets addressed to typed component IDs.
4. **Extensibility.** Adding a core, a cache level, a memory controller, a
   coherence protocol, an interconnect or a device is a new module plus a
   config entry. The topology is data, not code.

## Where gem5 stops and this design continues

gem5's classic memory system couples caches to a snooping crossbar and
encodes coherence in the cache itself; Ruby fixes that with SLICC but at
the cost of a separate language and a build step. Here the three concerns
are separate Rust types that compose:

- a **protocol** (`CoherenceProtocol`): a pure state machine over per-line
  states (MSI, MESI, MOESI, ...), testable without any timing model;
- a **home agent** (`HomeAgent`): the policy that decides who must be
  snooped for a request (broadcast, snoop filter, full directory);
- a **transport** (`Interconnect`): the timing of moving a message from A
  to B (crossbar, ring, mesh), with buffers, bandwidth and virtual
  channels.

Any protocol works with any home agent on any transport. gem5 also ticks
its CPU models with per-object events; rvsim ticks pipelines
synchronously (one tick per core per cycle, in core order) and uses events
only between components, which keeps pipeline code straightforward and
makes the schedule trivially deterministic.

## System decomposition

```
Simulator
├── state: SimState                    system state, owned once
│   ├── harts:  Vec<Hart>              architectural state per hardware thread
│   ├── cores:  Vec<Core>              private micro-architecture per core
│   │                                  (L1I, L1D, L2, MSHRs, WCB, predictor)
│   └── shared: SharedState            the uncore
│       ├── topology: Topology         every component ID, derived from config
│       ├── clock: u64                 master cycle counter
│       ├── event_queue: EventQueue    single ordered queue for all components
│       ├── bus: Bus                   MMIO devices, RAM fast path, interrupt lines
│       ├── llc: Cache                 shared last-level cache
│       ├── mem_controllers: Vec<Box<dyn MemoryController>>
│       ├── reservations: ReservationSet   LR/SC reservations, one per hart
│       ├── config, stats, exit signal, per-hart debug bookkeeping
├── pipelines: Vec<PipelineDispatch>   one per core, indexed by CoreId
└── coherence: CoherenceFabric         home agent + transport (absent for one core)
```

### Ownership and the per-core view

Pipeline stages must see "my hart, my core, and the uncore" without being
able to touch another core. They receive a **view**:

```rust
pub struct CoreCtx<'a> {
    pub hart: &'a mut Hart,
    pub core: &'a mut Core,
    pub shared: &'a mut SharedState,
}
impl Deref for CoreCtx<'_> { type Target = SharedState; }
impl DerefMut for CoreCtx<'_> {}
```

The simulator builds one view per core per tick from disjoint borrows of
`harts[i]`, `cores[i]` and `shared`. Everything that used to be a method
on the single-hart state (`translate`, `trap`, `csr_read`, `csr_write`,
trigger checks, reservation handling) is a method on `CoreCtx`. A stage
cannot reach another core because no path exists from a view to another
view. The view derefs to the uncore so `ctx.event_queue`, `ctx.bus`,
`ctx.config` and `ctx.stats` read naturally.

SMT (several harts per core) fits without change: the view's `hart` is the
hart the pipeline is currently working on behalf of; `Core::hart_ids`
lists the residents.

### Topology and identifiers

`Topology` is built once from config and is the only place that assigns
component IDs:

- `CoreId(c)` → `PipelineId(c)`, `HartId`s hosted, `CacheId`s for L1I,
  L1D and L2 (`3c`, `3c+1`, `3c+2`).
- `CacheId(3N)` → the LLC. Adding a level or a slice is a change to
  `Topology`, not to dispatch code.
- `MemCtrlId(k)` per memory controller; address-interleaved by the bus.
- `ComponentId` is the union used for event addressing.

Request IDs are unique system-wide: `ReqId = (PipelineId << 48) | seq`
for pipeline-originated requests, so caches and memory controllers may key
their pending tables by request ID with no collision between cores.

### The cycle

Order is fixed and is the same for one core and for sixty-four:

1. **Uncore pre-cycle.** Exit and kernel-panic checks; devices tick once,
   producing an `IrqLines` entry per hart (CLINT `msip`/`mtip` per hart,
   PLIC `meip`/`seip` per hart context); the master clock advances.
2. **Per-hart pre-tick**, in hart order: interrupt lines into `mip`,
   `stimecmp` compare, hang detection, mode-cycle statistics.
3. **Drain** every event whose `fire_at` is the current cycle.
4. **Pipelines tick** in core order. Each pipeline drains its mailbox,
   runs its stages, and schedules requests at the current cycle or later.
5. **Drain** again so requests reach their first component this cycle.
6. **Uncore tick**: memory controllers, then the coherence fabric.
7. **Drain** again.
8. **Per-hart post-tick** in hart order (x0 reset, mode tracing).

Because every cross-component interaction is an event, and events are
delivered in (`fire_at`, `seq`) order, the relative order of core ticks
never leaks into timing except through the events they schedule, which is
exactly what a real system's arbitration does. A core that is stalled
costs nothing beyond its own tick.

### Clock domains

The DDR5 controller already runs on its own clock through `ClockRatio`.
The same type serves any component with a different clock: a component
converts the cycle it receives into its own clock, advances as many of its
own clocks as elapsed, and converts response times back. Per-core clocks
(DVFS, big.LITTLE) are a `ClockRatio` on the core's tick: the simulator
asks `ratio.to_dram(cycle)`-style "how many of my clocks are due" and
ticks the pipeline that many times. No component sees another's clock.

## Interrupts

`Bus::tick` returns `Vec<HartIrqs>`, one per hart. The CLINT owns `msip`
and `mtimecmp` per hart at the standard offsets (`0x0` + 4·hart, `0x4000`
+ 8·hart) and one `mtime`. The PLIC has two contexts per hart
(M and S) in the standard layout (context 2·hart, 2·hart+1). The device
tree lists one `cpu@N` per hart with its interrupt controller phandle, and
the CLINT and PLIC `interrupts-extended` properties enumerate every hart,
so OpenSBI and Linux enumerate all harts without changes.

Secondary harts start at the reset PC with `mhartid` set; the riscv-tests
environment parks them (`csrr a0, mhartid; bnez a0, .`), and OpenSBI's
HSM parks and later lifts them for Linux.

## Atomics and reservations

LR/SC reservations live in `SharedState::reservations`, one slot per hart,
because a store from *any* hart to a reserved line must invalidate that
reservation. At store commit the committing hart clears every other
hart's reservation covering the line (its own reservation is governed by
the LR/SC pairing rules that already exist). AMOs are executed at the
memory side of the L1D as today; with coherence enabled they require the
line in the Modified state first, so the coherence transaction precedes
the read-modify-write.

## Coherence

### Data versus timing

Functional data lives in one place: RAM, written at store drain and read
at load response. Caches hold tags, states and dirtiness, not data. This
is what makes the model deterministic and simple to reason about: the
coherence protocol changes *when* an access completes and *which* caches
still hold a line, never *what* value a load returns. Memory-ordering
semantics (RVWMO) are enforced by the load queue and store buffer per
core, as now.

### Roles

Following CHI terminology, which maps cleanly onto real designs:

- **Requesting agent (RA):** each core's private cache hierarchy, with the
  L2 as the interface to the fabric. The L1D is kept coherent with its L2
  by back-invalidation (inclusive at the coherence level).
- **Home agent (HA):** the point of coherence for an address, co-located
  with the LLC. It serialises requests to a line, decides which RAs must
  be snooped, and forwards data from an owner or from memory.
- **Interconnect:** carries request, snoop, response and data messages
  between RAs and the HA with modelled latency and bandwidth.

### Traits

```rust
pub trait CoherenceProtocol {
    type State: Copy + Eq + Default;          // e.g. MesiState
    fn on_request(&self, s: State, req: ReqKind) -> LocalAction;   // hit / need-upgrade / need-fill
    fn on_snoop(&self, s: State, snoop: SnoopKind) -> SnoopAction; // downgrade / invalidate / supply
    fn after_fill(&self, req: ReqKind, others_had_copy: bool) -> State;
}

pub trait HomeAgent {
    fn on_request(&mut self, line: LineAddr, from: CoreId, req: ReqKind) -> HomeDecision;
    fn on_snoop_response(&mut self, line: LineAddr, from: CoreId, resp: SnoopResp);
    fn on_eviction(&mut self, line: LineAddr, from: CoreId);
}
// impls: Broadcast (snoop everyone), SnoopFilter (bitmap of possible sharers), Directory (exact sharers + owner)

pub trait Interconnect {
    fn send(&mut self, now: u64, msg: CoherenceMsg) -> ();
    fn tick(&mut self, now: u64) -> impl Iterator<Item = CoherenceMsg>;   // delivered this cycle
    fn topology(&self) -> TopologyInfo;
}
// impls: Crossbar (fixed latency, per-port bandwidth), Ring, Mesh (XY routing), Hypercube
```

A transaction at the HA is a small state machine (a *transaction buffer*
entry, like a CHI request tracker): request received → snoops sent →
responses collected → data forwarded → done. Entries are the natural place
for the stats real designs expose (snoops per request, cache-to-cache
transfers, latency histograms). Line-level serialisation at the HA
guarantees a single writer at any time; the protocol's invariants
(at most one Modified/Exclusive holder, Shared only with an up-to-date
memory or an Owner) are checked in debug builds after every transition.

### Default configuration

MESI protocol, snoop-filter home agent at the LLC, crossbar interconnect
with a configurable hop latency and per-port bandwidth. Single-core
configurations bypass the fabric entirely and stay cycle-identical to
today.

### Growth paths

- MOESI: a new `CoherenceProtocol` with an Owned state; the HA forwards
  from the owner instead of writing back.
- Directory at scale, hierarchical clusters, token coherence: new
  `HomeAgent` impls; a cluster is a `HomeAgent` that wraps two agents.
- Ring / mesh / hypercube: new `Interconnect` impls, no protocol change.
- Virtual channels: an interconnect concern; message classes are already
  distinct enum variants so deadlock-freedom can be enforced per channel.
- Memory consistency experiments: a `ConsistencyModel` gate in the load
  queue, orthogonal to coherence.

## Statistics

Every core's counters are rooted at `core<N>` and every hart's at
`hart<N>`; the fabric reports under `coherence.*` (per-HA and per-link),
memory controllers under `memctrl<N>`. Paths are allocated once per
component at construction, as the DDR5 controller does, so the hot path
increments a counter through a pre-resolved `&'static str`.

## Implementation stages

Each stage is a set of small commits that leaves the tree working, with
single-core configurations cycle-identical to the previous stage.

1. **Arenas and views.** `SimState` becomes `harts` + `cores` + `shared`;
   `CoreCtx` replaces the single-hart state in every stage; `Topology`
   assigns IDs; request IDs carry their pipeline; reservations and
   instruction counts move to where multi-hart semantics need them; the
   simulator ticks `Vec<PipelineDispatch>`.
2. **Multi-hart uncore.** Per-hart CLINT and PLIC contexts, per-hart
   interrupt lines, device tree enumeration, secondary hart reset, cross
   hart reservation invalidation; a two-hart synchronisation test.
3. **Coherence fabric.** Protocol, home agent and interconnect traits with
   MESI, snoop filter and crossbar; coherence states in private caches;
   invariant checks; stats; litmus-style tests.
4. **Exposure.** Python `Config(hart_count=N)` plus per-hart accessors,
   per-core statistics paths, documentation.
