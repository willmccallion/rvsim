# Stats & Observability

rvsim exposes every microarchitectural counter through a single hierarchical
tree with path-addressed access, per-counter metadata, wildcard queries,
first-class derived metrics and snapshots that measure a region. This page
lists what is counted, shows how to read it from Python, and records the
reasoning behind the design so future changes stay coherent with it.

## Motivation

We started with gem5-shaped stats (a flat text dump per run, no query language,
no metadata) and hit the same friction gem5 users hit:

- **Class-name leakage in paths.** Swapping `TournamentBP` for `TAGE_SC_L` in
  the config rewrites every stat path. Scripts break for a reason that has
  nothing to do with the workload.
- **Inconsistent naming across subsystems.** One cache reports `hits`, another
  reports `read_hits + write_hits + prefetch_hits`. No way to write "miss rate
  across all L1s" without special-casing each level.
- **No query language.** Summing a counter across cores means grepping the text
  dump in Python. Wildcards are the user's problem.
- **No metadata.** The dump is `name value`. Units, whether a counter is a
  gauge vs an accumulator, and whether it makes sense to sum are all folklore.
- **No standard summary.** Every team writes their own `parse_stats.py`. The
  five most useful numbers for a run are buried 200 lines into the file.
- **Backend divergence.** The InOrder and O3 backends had subtly different
  counter names for the same event because they were plumbed independently.

The stats layer is built to fix these directly. The design goals below
each map to one of these pain points.

## Path grammar

Every counter has a path of the form:

```
<subject>.<subsystem>[.<sub>].<counter>
```

Read left-to-right it should parse as an English sentence:
`core0.pipeline.stalls.control` — "on core 0, in the pipeline, control-flow
stalls."

### Top-level subjects

```
core<N>       — physical execution core (pipeline + private caches + BP)
hart<N>       — architectural hart (retired instructions, traps, mode cycles)
llc           — the shared last-level cache (registered, and zero, without an L3)
memctrl<N>    — DDR5 memory controller channels and sub-channels (DDR5 only)
coherence     — coherence fabric: coherence.ha.* (home agent), coherence.interconnect.*
               (only with more than one core)
system        — sums over every hart (system.retired_insts, system.traps)
```

### Why no `system.` prefix

gem5 prefixes everything with `system.` because a gem5 config can technically
contain multiple systems. In practice nobody uses that, and the prefix just
adds noise to every path and every grep. We drop it: the whole tree *is* the
system, and the top-level subjects speak for themselves.

### Why `core<N>` and `hart<N>` — not `cpu<N>`

"CPU" is ambiguous. gem5 uses it for the pipeline; RISC-V uses "hart" for the
architectural state and "core" for the physical execution engine. The Rust
side has no `Cpu` type either: a `Hart` holds architectural state and a
`Core` the micro-architecture. A `cpu<N>` namespace label would undo that
clarification.

Splitting `core<N>` and `hart<N>`:

- `hart<N>.*` — architectural counters that need to survive an SMT change
  (retired instructions, traps, privilege-mode cycles). These drive
  `MINSTRET`, `MCYCLE`, etc.
- `core<N>.*` — microarchitectural counters that belong to the physical engine
  (pipeline stalls, cache hits, branch prediction, functional-unit
  utilization).

Under SMT (deferred) a single `core<N>` will host multiple `hart<N>` subjects.
Keeping harts at the top level rather than nesting them under cores means the
query `hart*.retired_insts` keeps working across configuration changes.

### Why subjects are enumerated by index, not by class name

`core<N>` never becomes `o3_core<N>` or `inorder_core<N>` even though the
backends are different code. Scripts stay valid across a backend swap, which is
one of the main things you want to compare between runs. This is the biggest
single fix versus gem5.

### Predictable stems per subject

Every `core<N>` exposes the same subsystem set (`commit`, `pipeline`, `bp`,
`cache.l1i`, `cache.l1d`, `mdp`, `wcb`, `fu`, ...) and each subsystem exposes
the same counter names across cores. Every cache — regardless of level — has
`.hits` and `.misses`. One analysis script works for every level and every
core. gem5's per-cache-class naming is what breaks this today.

## Catalogue

Every path below exists on every run that has the component, and reads
zero until something counts. `<N>` is a core or hart index and `<level>`
one of `l1i`, `l1d` and `l2`.

### Per hart

| Path | Meaning |
|------|---------|
| `hart<N>.retired_insts` | Instructions the hart retired |
| `hart<N>.traps` | Traps taken, exceptions and interrupts |
| `hart<N>.cycles.user`, `.kernel`, `.machine` | Cycles spent in U, S and M mode |

### Per core: pipeline

| Path | Meaning |
|------|---------|
| `core<N>.ipc`, `core<N>.cpi` | Derived: the core's hart's retired instructions over the core's cycles, and the inverse |
| `core<N>.pipeline.cycles.total` | Cycles the core was ticked or counted |
| `core<N>.pipeline.cycles.wfi` | Cycles waiting in `WFI` |
| `core<N>.pipeline.cycles.rob_empty` | Cycles with an empty ROB |
| `core<N>.pipeline.stalls.control` | Cycles from a backend redirect (misprediction, trap, re-execution) until rename hands on the first instruction from the new path |
| `core<N>.pipeline.stalls.fetch_wait` | Cycles fetch waited for an in-flight I-cache access |
| `core<N>.pipeline.stalls.data` | Cycles nothing issued and the oldest queued instruction waited on its operands |
| `core<N>.pipeline.stalls.ordering` | Cycles nothing issued and the oldest queued instruction was held for program order: it executes only as the oldest (system, vector, AMO/SC), waits on a fence or an older store's address, or a pending squash removes it |
| `core<N>.pipeline.stalls.fu_structural` | Cycles a ready instruction waited for a free unit |
| `core<N>.pipeline.stalls.backpressure` | Cycles issue held because memory1 still held ops behind a page-table walk |
| `core<N>.pipeline.stalls.dispatch` | Cycles rename had no ROB, issue-queue, load-queue or store-buffer room |
| `core<N>.pipeline.stalls.checkpoint` | Cycles rename waited for a free branch checkpoint |
| `core<N>.pipeline.stalls.serialize` | Cycles rename waited behind a serializing instruction |
| `core<N>.pipeline.stalls.squash` | Cycles rename waited while commit squashed the ROB |
| `core<N>.pipeline.flushes.total` | Flushes taken at execute or at commit, each under exactly one of `.branch` (misprediction), `.system` (a system instruction, CSR access or vector op refetching what follows, or commit refetching after xRET, FENCE.I, SFENCE.VMA, a WFI wake or a store whose PTE changed after its walk), `.mem_violations` (a load that read past an aliasing store), `.coherence` (a value another hart overwrote) and `.trap` (an exception or interrupt taken at commit) |
| `core<N>.pipeline.flushes.squashed_insns` | ROB entries those flushes dropped |
| `core<N>.commit.op.{alu,branch,load,store,atomic,system}` | Retired scalar integer instructions by kind; `atomic` is LR, SC and the AMOs, which `load` and `store` leave out |
| `core<N>.commit.fp.{arith,fma,div_sqrt,load,store}` | Retired floating-point instructions by kind |
| `core<N>.commit.vec.{int,fp,load,store,misc,crypto}` | Retired vector instructions by kind; `int` includes the Zvbb/Zvbc bit-manipulation ops, `misc` is permute, mask and configuration, `crypto` the Zvk* ops |
| `core<N>.commit.retire_histogram.{zero,one,two,three_plus}` | Cycles by the number of instructions retired in them |
| `core<N>.fu.util.<unit>` | Use of each functional-unit type (`int_alu`, `int_mul`, `fp_fma`, `vec_permute`, ...) |

### Per core: prediction and memory ordering

| Path | Meaning |
|------|---------|
| `core<N>.bp.committed.hits`, `.mispredicts`, `.accuracy` | Control instructions predicted right and wrong, counted at commit; accuracy is derived |
| `core<N>.bp.spec.hits`, `.mispredicts`, `.accuracy` | The same counted at execute, wrong-path branches included |
| `core<N>.bp.decode_redirects` | Fetch redirects decode made (a BTB miss on a taken control instruction, a stale target, a non-branch BTB hit) |
| `core<N>.mdp.predictions.bypass` | Loads predicted independent of every older store |
| `core<N>.mdp.predictions.wait_all` | Loads made to wait for every older store's address: all loads under the blind predictor, LRs and AMOs under store sets |
| `core<N>.mdp.predictions.wait_for` | Loads and stores made to wait for one older store in their store set |
| `core<N>.mdp.violations` | Loads found to have read past an aliasing older store, which the predictor trained on |
| `core<N>.lsq.rescheduled_mem_ops` | Memory ops that waited in memory1, each counted once per wait: for an older store (a partial overlap, or data not yet there), a device read or AMO/SC waiting to be the oldest, or a PTE's D bit |
| `core<N>.lsq.split_stores` | Stores whose data half issued after their address half |
| `core<N>.lsq.coherence_replays` | LRs and AMOs re-executed after another hart wrote their line |
| `core<N>.lsq.coherence_violations` | Loads squashed for reading a line before a remote write an older load saw |
| `core<N>.wcb.coalesces`, `.drains` | Stores merged into a line the write-combining buffer already held, and lines it wrote to the L1D |
| `core<N>.prefetch.loads.l1`, `.l2` | Load prefetches the load/store unit sent to fill the L1D, and to fill the L2 alone |
| `core<N>.prefetch.loads.dropped.page_boundary`, `.tlb_miss`, `.denied`, `.not_ram` | Load prefetches not sent: past the trained page under `PageBoundary.Stop()`, next page not in the data TLB, a page or region the load may not read, not RAM |

### Caches

Every cache, `core<N>.cache.<level>` and `llc`, has the same counters:

| Path | Meaning |
|------|---------|
| `hits`, `misses`, `miss_rate` | Demand accesses that hit and missed; the rate is derived |
| `mshr_hits` | Misses that joined a fetch already in flight |
| `blocked_requests` | Requests that waited because the MSHRs or writeback buffer were full |
| `fills`, `evictions`, `writebacks` | Lines installed, lines evicted, and dirty lines written to the next level |
| `back_invalidations` | Lines invalidated because an inclusive level below evicted them |
| `prefetches.issued` | Prefetch fetches this cache started |
| `prefetches.late` | Prefetch fetches a request from above joined while still in flight |
| `prefetches.useful` | Prefetched lines a request from above found once installed (counted on the first such request) |
| `prefetches.unused` | Prefetched lines evicted, snooped away or invalidated before any request found them |
| `prefetches.used`, `prefetches.accuracy` | Derived: `late + useful`, and `used / issued` |

`useful` and `unused` count what gem5's `pfUseful` and `pfUnused` count. `late` and
`accuracy` follow Srinath et al., "Feedback Directed Prefetching" (HPCA 2007): a
prefetch is late when a demand arrives before it completes, and accuracy counts
late prefetches as used. gem5's `pfLate` is different (prefetch candidates dropped
because the line was already held, in flight or being written back), and its
`accuracy` is `pfUseful / pfIssued`.
| `prefetches.page_crossing` | Candidates of this cache's own prefetcher dropped for lying outside the 4 KiB page of the access that produced them |
| `prefetches.dropped` | Prefetch requests from above dropped rather than take the last free MSHR |
| `prefetches.store_stream` | Prefetches the L1D's store-miss prefetcher sent to the L2 |
| `probes` | Lookups made for another agent (snoops, inclusive back-invalidations) |
| `maintenance` | Cache-block operations applied to this level |
| `coherence.snoops`, `.invalidations`, `.downgrades`, `.upgrades`, `.upgrade_retries` | Coherence traffic this cache answered or caused (zero on one core) |

The L1D also counts `exclusive_swaps`: lines handed to the L2 under the
exclusive inclusion policy.

### Shared components

| Path | Meaning |
|------|---------|
| `coherence.ha.requests.{read_shared,read_unique,clean_unique,writebacks,evicts,stale_writebacks,maintenance,non_coherent}` | Requests the home agent received, by kind |
| `coherence.ha.snoops_sent`, `.c2c_transfers`, `.recalls` | Snoops sent, lines supplied by another cache, snoop-filter recalls |
| `coherence.ha.filter.hits`, `.misses` | Snoop-filter lookups |
| `coherence.ha.serialised`, `.txn_full_stalls` | Requests that waited behind another on the same line, or for a free transaction |
| `coherence.interconnect.{messages,bytes,busy_cycles,blocked_cycles}` | Interconnect traffic and occupancy |
| `memctrl0.ch<C>.sc<S>.*` | DDR5 sub-channel counters and histograms (see [Memory Hierarchy](memory.md#ddr5-controller)); per-bank counters under `rank<R>.bank<B>` |
| `system.retired_insts`, `system.traps` | Sums over every hart |

The run-level `cycles`, `instructions_retired` and `ipc` are not paths in
the tree: they are properties of the stats object (and keys of
`result.stats`, below).

## Reading statistics from Python

There are two views of the same tree.

**The live tree.** `Simulator.stats` returns a snapshot of the native tree.
It takes wildcard queries and prints the summary:

```python
from rvsim import Simulator, Config

cpu = Simulator(Config(), binary="software/bin/programs/qsort.elf")
cpu.run()

stats = cpu.stats
stats["core0.ipc"]                              # one stat; KeyError if unknown
stats.get("core0.cache.l1d.misses", 0.0)        # or a default
stats.query("core*.cache.l1d.misses").sum()     # sum across cores
stats.query("**.misses").by_subject()           # {"core0": ..., "llc": ...}
for path, value in stats.query("core0.pipeline.stalls.*"):
    print(path, value)
print(stats.summary(["core0", "hart0"]))        # the formatted summary
stats.subjects()                                # ["core0", "hart0", "llc", "system"]
```

`*` matches within one segment and `**` any number of segments.

**The flat dictionary.** `Environment.run()`, `Sweep` and `Session`
return `Stats`, a `dict` keyed by path with the run-level `cycles`,
`instructions_retired` and `ipc` added. Its `query()` takes a regular
expression (or a substring), which suits interactive filtering, and
`Stats.tabulate()` lays several runs side by side:

```python
result.stats["core0.cache.l1d.misses"]
result.stats.query(r"l1d\.(hits|misses)$")
```

## Measuring a region

A whole run includes start-up and shutdown. Three ways measure only the
part of interest, leaving the whole run's statistics intact:

- **Subtract snapshots.** `stats - earlier` subtracts every counter and
  recomputes derived stats from the differences:

    ```python
    start = cpu.stats
    cpu.run_until(pc=0x80001234)
    region = cpu.stats - start
    print(region.ipc, region["core0.bp.committed.accuracy"])
    ```

- **Let the guest mark it.** Software writes to the
  [sim-control device](soc.md#sim-control) to dump labelled snapshots;
  `cpu.stats_dumps()` returns them and `cpu.stats_between(start, end)`
  subtracts a pair. `Session.measure()` and `rvsim bench` measure a Linux
  command this way, bracketing its whole process lifetime.
- **Reset.** `cpu.reset_stats()`, or the guest's reset command, zeroes the
  tree, as gem5's `m5 resetstats` does. Subtracting snapshots is usually
  better, since it keeps the whole-run numbers.

Histograms subtract exactly in count, sum and mean, but a region's
histogram has no minimum or maximum.

## Saving statistics

`rvsim program.elf --json out.json` writes the flat dictionary;
`rvsim bench --json` writes every measured region with its output. In Rust,
`Stats::dump` writes `path value` lines, with each histogram's count, sum,
mean, minimum and maximum.

## Path structs per component

Paths are declared once, as fields of a struct per subject in
`sim/stats/paths.rs`, and allocated when the component is built:

```rust
stat_paths! {
    CommitPaths {
        op_load: "commit.op.load",
        op_store: "commit.op.store",
        ...
    }
}
```

`CorePaths::new(CoreId(3))` leaks `"core3.commit.op.load"` and friends once;
the `Core` keeps the struct, `Uncore` keeps one `HartPaths` per hart.
Writers use the field, not a string:

```rust
state.uncore.stats.counter(state.core.stat_paths.commit.op_load).inc();
```

Typos become compile errors instead of silently-zero counters — which is
exactly the failure mode the earlier flat stats hit (cache/MSHR/WCB counters
had been printing zero for months because the writers had drifted out of sync
with the field names) — and the hot path still increments through a pre-resolved
`&'static str`. `Stats::for_components` registers every hart's and core's
paths with their metadata from the topology, so `core1.commit.op.load` exists
(and is zero) on a two-core system before anything has retired, and derives
the `system.*` sums (`system.retired_insts`, `system.traps`) over every hart.

## Per-counter metadata

Every counter registers a `Meta` at simulator build time:

```rust
pub struct Meta {
    pub desc: &'static str,
    pub unit: Unit,       // Events, Cycles, Bytes, Ratio, Rate, Percent
    pub kind: Kind,       // Accumulated, Gauge, Rate
}
```

Metadata unlocks three things that would otherwise stay in prose:

- **Auto-summary.** `stats.summary()` walks metadata and formats units
  correctly. No hardcoded print statements per counter.
- **Aggregation safety.** Summing gauges across cores is nonsense; summing
  accumulators is fine. `Kind` tells the query layer which is which.
- **Documentation at the source.** `desc` lives next to the counter, not in a
  separate `stats.md` table that rots. `stats.summary()` can print it on
  demand.

Registering at build time (not lazily on first write) means the tree shape is
known before any run — you can enumerate available stats without executing a
workload. That's what makes `stats.summary()` deterministic and what makes a
future GUI/notebook completion tractable.

## Query language

```rust
stats.query("core*.commit.insts").sum()
stats.query("**.misses").iter()
stats.query("core0.cache.**.hits").by_subject()
```

Two wildcards:

- `*` matches exactly one path segment.
- `**` matches any depth.

This is intentionally not a full expression language. The goal is aggregation
across parallel subjects (all cores, all cache levels, all memctrls), which the
two-wildcard pattern language covers. Arithmetic across queries lives in
Python, where the user already has pandas and numpy.

Malformed patterns are errors, not silent empty results — a wrong pattern is
almost always a typo, and returning zero would recreate the "counter silently
reads zero" problem that started this refactor.

## Derived metrics as first-class stats

IPC, CPI, prediction accuracy, and miss-rate are registered like any other
counter, with a formula:

```rust
s.derive(
    c.ipc,
    Formula::Div(first_hart.retired_insts, pipe.cycles_total),
    Meta::ratio("instructions per cycle"),
);
```

`stats["core0.ipc"]` just works. `stats.summary()` prints it under the core's
section. Every consumer (Python API, text dump, future JSON export) gets the
same value from the same source. No per-team `parse_stats.py` computing IPC a
slightly different way.

The formula language is deliberately small (`Div`, `Ratio`, `Sum`) — enough for
IPC/CPI/accuracy/miss-rate and nothing more. Anything more elaborate is a
Python problem.

Divide-by-zero returns `0.0`, not `NaN`: users comparing runs across
configurations don't want `NaN` poisoning their spreadsheets when a counter is legitimately zero (e.g.,
`bp.accuracy` for a workload with zero branches).

## Auto-generated summary

`stats.summary()` prints the run-level totals and then one section per
subject. Sections come from grouping metadata by subject; alignment, unit
formatting and derived-metric placement fall out of the metadata. Adding a
counter is a one-line change and the summary picks it up automatically;
there is no hand-written formatter to keep in step.

Components outside the core (caches, the coherence fabric, the memory
controller) implement `StatSource` and register their own paths when the
system is built, so the kernel's statistics module never lists them.

## No flat aliases

There are no flat aliases such as `dcache_misses` or `branch_accuracy_pct`: a
second vocabulary would drift from the tree the same way the old counters
did. Comparisons (`Result.compare`, `Sweep.run().compare`) take paths and read
a metric's direction and aggregation from its last segment: `ipc`,
`accuracy` and `hits` are better higher; `cycles`, `misses`, `mispredicts`,
`miss_rate` and every `stalls.*` counter better lower; an `accuracy` or
`miss_rate` over several runs is recomputed from the counters beside it.

## What this does not solve

Explicitly out of scope for this design, so future contributors don't try to
squeeze them in:

- **Time series.** A region is the difference of two snapshots; there is no
  per-interval sampling of the tree.
- **Full arithmetic query language** — Python is a better place for this.
- **Native JSON or CSV export** from the tree; the CLI writes JSON from the
  flat dictionary.
- **Per-thread histograms under SMT** — waits for the SMT hart layout.

## Summary

| gem5 pain point | Design response |
|---|---|
| Class names leak into paths | Subjects are indexed (`core0`), never class-named |
| Names diverge across subsystems | Same stem per subject; every cache has `hits`/`misses` |
| No query language | `*`/`**` wildcards with `sum` / `by_subject` |
| No metadata | Every counter has `desc`/`unit`/`kind` at registration |
| No standard summary | `stats.summary()` walks metadata, no hardcoded formatter |
| Backends diverge | One shared writer path in `Handle` impls; both backends use the same paths |
| Typos silently zero | Const path declarations — misspell means compile error |
| Derived metrics are per-user | `stats.derive(...)` registers IPC/CPI/accuracy once |
| `system.` prefix noise | Dropped — the tree is the system |
| `cpu` vs `core` vs `hart` ambiguity | `core<N>` for microarch, `hart<N>` for architectural |
