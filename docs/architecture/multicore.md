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
able to touch another core, and only commit may change architectural
state. They receive one of two **views**:

```rust
pub struct CoreCtx<'a> {              // commit, traps, the engine's redirects
    pub hart: &'a mut Hart,
    pub core: &'a mut Core,
    pub shared: &'a mut SharedState,
}
impl Deref for CoreCtx<'_> { type Target = SharedState; }
impl DerefMut for CoreCtx<'_> {}

pub struct StageCtx<'a> {             // fetch, decode, rename, issue, execute, memory, writeback
    hart: &'a Hart,                   // read-only
    core: &'a mut Core,               // TLBs, predictor, caches
    shared: &'a mut SharedState,      // only `counter()` and `events()` are exposed
}
impl Deref for StageCtx<'_> { type Target = SharedState; }
```

The simulator builds one `CoreCtx` per core per tick from disjoint borrows
of `harts[i]`, `cores[i]` and `shared`; the engine hands each stage a
`StageCtx` derived from it. Translation, CSR reads and trigger checks are
methods on both; `trap`, `csr_write`, register writes, `publish_write`
and reservation handling exist only on `CoreCtx`, so an execute-stage
write to architectural state does not compile. A stage cannot reach
another core because no path exists from a view to another view. Both
views deref to the uncore so `ctx.bus`, `ctx.config` and `ctx.cycle`
read naturally.

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
reservation. A published write breaks every other hart's reservation
covering the line (the writer's own reservation is governed by the LR/SC
pairing rules that already exist): at drain for plain stores, at commit
for SC and AMO. AMOs read their old value at the memory side of the L1D as
today and apply the result at commit after the write-log check above;
with coherence enabled they require the line in the Modified state first,
so the coherence transaction precedes the read.

## Coherence

### Data versus timing

Functional data lives in one place: RAM, written when a store is
published and read at load response. Caches hold tags, states and
dirtiness, not data. This is what makes the model deterministic and simple
to reason about: the coherence protocol changes *when* an access completes
and *which* caches still hold a line, never *what* value a load returns.
Memory-ordering semantics (RVWMO) are enforced by the load queue and store
buffer per core, as now.

### Publishing writes and the write log

Because caches hold no data, a hit on a line that another hart has just
written returns the new RAM value even though the invalidation has not
arrived yet, and a value read at load response can be overwritten by
another hart before the reading instruction commits. Neither can be
detected by the protocol, so the pipeline makes the visibility instant
explicit:

- Every RAM write goes through `SharedState::publish_write`: the bytes
  land in RAM, every other hart's reservation on the line is broken, and
  the write is recorded in the **write log**, a per-line `(sequence,
  writer)` table that exists only when the system has more than one hart.
  Plain stores publish when they drain from the store buffer (a store
  buffer is invisible to other harts, as in hardware); SC and AMO publish
  at commit, at the same instant as the reservation decision, and their
  store-buffer entry then drains as timing only.
- A load, LR or AMO response is stamped with the log sequence current
  when its bytes were read.
- At commit an LR whose line another hart has written after its stamp
  re-executes (everything from it is squashed and refetched), so a
  reservation is never set on a stale value. An AMO re-executes only if
  another hart wrote its line *and* the word it read has changed: a real
  core holds the line for its read-modify-write, so a write elsewhere in
  the line, or one that restored the same value, must not perturb it, and
  replaying on every line write lets harts contending for one lock word
  replay each other forever. Plain loads are not replayed in the in-order
  pipeline: RVWMO lets them keep the earlier value.
- In the out-of-order pipeline a younger load that executed before an
  older load to the same line is squashed when the older load's response
  shows the line was written by another hart in between, the same rule
  gem5's LSQ applies on an external snoop (per-location coherence, CoRR).

With one hart the log does not exist and no stamp is ever compared, so
single-core behaviour is unchanged.

### Roles

Following CHI terminology, which maps cleanly onto real designs:

- **Requesting agent (RA):** each core's private cache hierarchy, with the
  L2 as its interface to the fabric. With more than one core the L2 is
  made inclusive of its L1s so that a snoop can be answered from its tags;
  each L2 line carries presence bits for the L1s it was handed to, so a
  snoop probes only those (gem5's snoop filter plays the same role), and
  one nobody above holds is answered from the L2 alone. A disabled L2
  still acts as the agent for the L1s above it, and a core
  with no private cache at all takes no part in coherence (its accesses
  cross the fabric as non-snooped memory accesses).
- **Home agent (HA):** the point of coherence for an address, co-located
  with the LLC. It serialises requests to a line, decides which RAs must
  be snooped, and forwards data from an owner or from memory.
- **Interconnect:** carries request, snoop, response and data messages
  between RAs and the HA with modelled latency and bandwidth.

### Messages

Only the L2 ↔ home boundary speaks coherence messages (`Packet::Coh`);
L1s and L2 keep exchanging `MemReq`/`MemResp`, whose responses carry the
granted state. The vocabulary is CHI's:

| Message | Direction | Meaning |
|---|---|---|
| `ReadShared`, `ReadUnique` | RA → HA | fetch a line to read / to write |
| `CleanUnique` | RA → HA | write permission for a line held Shared |
| `WriteBack{dirty}`, `Evict` | RA → HA | a victim leaves the L2 (dirty data, or a clean copy so tracking stays exact) |
| `SnpShared`, `SnpUnique`, `SnpInvalid` | HA → RA | keep at most a shared copy / drop the line / drop it for a recall |
| `SnoopResp{had_copy, dirty}` | RA → HA | what the RA held |
| `CompData{state}`, `Comp{state}` | HA → RA | completion with / without data |
| `CompAck` | RA → HA | the completion was taken up |
| `NoSnp`, `NoSnpData` | RA ↔ HA | an access outside coherence (a core without caches) |

Each message belongs to one of four classes (request, snoop, response,
data) that travel on separate virtual channels, so a response never waits
behind a request. The home does not start the next transaction on a line
until the requester's `CompAck` arrives, which is what lets an RA treat a
snoop that arrives while it has a request outstanding for the line as
ordered *before* that request: it answers from its current tags and the
fill that follows is authoritative. Inside a core the L2 forwards probes
to its L1s with the same delay as its responses, so the same rule holds
one level up.

Two consequences of that ordering are handled explicitly: a permission
grant (`Comp` for a `CleanUnique`) that arrives after a snoop took the
line is acknowledged and the fetch re-issued as `ReadUnique`; a
`WriteBack` from a core whose line a snoop already collected is only
acknowledged, its data having travelled with the snoop response.

### Traits

```rust
pub trait CoherenceProtocol {           // pure state machine; impl: Mesi
    fn snoops_for(&self, kind: ReqKind, requester: CoreId, holders: Holders) -> Vec<(CoreId, SnoopKind)>;
    fn grant(&self, kind: ReqKind, others_remain: bool) -> MesiState;
    fn after_snoop(&self, current: MesiState, snoop: SnoopKind) -> MesiState;
}

pub trait HomeAgent {                   // who holds a line; impls: Broadcast, SnoopFilter
    fn holders(&mut self, line: LineAddr, ..) -> Option<Holders>;   // None: snoop everyone
    fn room_for(&self, line: LineAddr, in_flight: &[LineAddr]) -> Room; // Available / Recall(victim) / AllBusy
    fn on_grant(..); fn on_release(..); fn on_downgrade(..);
}

pub trait Interconnect {                // impls: Crossbar, RoutedNetwork<Ring | Mesh2D | Hypercube>
    fn send(&mut self, now: u64, from: Node, msg: CoherenceMsg);
    fn tick(&mut self, now: u64, stats: &mut Stats, deliver: &mut dyn FnMut(Node, CoherenceMsg));
    fn topology(&self) -> TopologyInfo;
}
```

`CoherenceFabric` composes the three into the component the L2s talk to
(`ComponentId::Fabric`). A transaction at the home is a small state
machine, like a CHI request tracker: request admitted → (victim recalled
to free tracking room) → snoops sent → responses collected → data fetched
from the LLC or forwarded from the owner that answered dirty (whose data
is written into the LLC at the same time) → completion sent → acknowledged.
One transaction is live per line; later requests for the line queue
behind it, which is the serialisation point that gives a single writer at
any time. The LLC is an ordinary cache level the home reads and writes
with `MemReq` packets.

The **snoop filter** keeps an exact sharer bitmap and owner per tracked
line in a set-associative array sized as a multiple of the aggregate
private L2 capacity; when a set is full the least recently used line not
in a live transaction is recalled (every holder invalidated) before the
new line is tracked, as Arm's snoop filter and AMD's probe filter do. An
untracked line with a live transaction already claims a way, so two
concurrent misses to a full set recall two victims. **Broadcast** tracks
nothing and snoops every other core.

The **crossbar** queues messages per input port and class, arbitrates
oldest-first for each output port and charges a hop latency plus the
transfer time of the message at the port's bandwidth. The **routed
networks** place the cores and the home on the nodes of a ring (shortest
direction), a square mesh or torus (XY routing) or a hypercube
(dimension-order routing); a message pays the hop latency and the link's
transfer time at every hop, and each link carries one message per class
at a time.

### Invariants

`coherence::audit` checks the whole system: for every line not in the
middle of a transaction, at most one L2 holds it Modified or Exclusive and
then no other L2 holds it; an L1 never holds a line its L2 does not, nor
in a stronger state; the snoop filter's sharers and owner match what the
L2s hold and it tracks no line nobody holds; no cache has duplicate tags.
The multi-hart integration tests run the audit every few cycles on every
home agent and interconnect.

### Default configuration

MESI protocol, snoop-filter home agent (1.5× the private L2 lines, 8
ways), crossbar interconnect with a 2-cycle hop and 32 bytes per cycle
per port, 32 transaction entries. Single-core configurations have no
fabric: the L2 talks to the LLC directly and stays cycle-identical.

### Growth paths

- MOESI: a new `CoherenceProtocol` with an Owned state; the HA forwards
  from the owner instead of writing back.
- Directory at scale, hierarchical clusters, token coherence: new
  `HomeAgent` impls; a cluster is a `HomeAgent` that wraps two agents.
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
   `CoreCtx` (later narrowed to `StageCtx` for every stage but commit)
   replaces the single-hart state in every stage; `Topology`
   assigns IDs; request IDs carry their pipeline; reservations and
   instruction counts move to where multi-hart semantics need them; the
   simulator ticks `Vec<PipelineDispatch>`.
2. **Multi-hart uncore.** Per-hart CLINT and PLIC contexts, per-hart
   interrupt lines, device tree enumeration, secondary hart reset, cross
   hart reservation invalidation; a two-hart synchronisation test.
3. **Coherence fabric.** Protocol, home agent and interconnect traits with
   MESI, broadcast and snoop-filter homes, crossbar, ring, mesh, torus and
   hypercube; coherence states in private caches; invariant audit; stats;
   audited multi-hart tests.
4. **Exposure.** Python `Config(hart_count=N)` plus per-hart accessors,
   per-core statistics paths, documentation.
