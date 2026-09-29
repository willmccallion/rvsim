# Stats & Observability

rvsim exposes every microarchitectural counter through a single hierarchical
tree with path-addressed access, per-counter metadata, wildcard queries, and
first-class derived metrics. This document captures the reasoning behind that
design so future changes stay coherent with the original intent.

The concrete path names, commit boundaries, and migration mechanics live in
`plan.md` and the Phase 3b plan. This page is about *why* the system looks the
way it does.

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

Phase 3b rebuilds the stats layer to fix these directly. The design goals below
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
hart<N>       — architectural hart (regs, CSRs, retired-inst counter)
memctrl<N>    — memory controller channels
bus           — interconnect
coherence     — coherence fabric: coherence.ha.* (home agent), coherence.interconnect.*
```

### Why no `system.` prefix

gem5 prefixes everything with `system.` because a gem5 config can technically
contain multiple systems. In practice nobody uses that, and the prefix just
adds noise to every path and every grep. We drop it: the whole tree *is* the
system, and the top-level subjects speak for themselves.

### Why `core<N>` and `hart<N>` — not `cpu<N>`

"CPU" is ambiguous. gem5 uses it for the pipeline; RISC-V uses "hart" for the
architectural state and "core" for the physical execution engine. Phase 3
already removed the `Cpu` Rust type in favor of `SystemState` (and, in Phase 5,
`Core` + `Hart` structs). Reintroducing `cpu<N>` as a namespace label would
undo that clarification.

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
state.shared.stats.counter(state.core.stat_paths.commit.op_load).inc();
```

Typos become compile errors instead of silently-zero counters — which is
exactly the failure mode Phase 2 hit (cache/MSHR/WCB counters had been printing
zero for months because the writers had drifted out of sync with the field
names) — and the hot path still increments through a pre-resolved
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
stats.derive(
    paths::core::IPC,
    Formula::Div(paths::hart::RETIRED_INSTS, paths::pipeline::CYCLES),
    Meta { desc: "instructions per cycle", unit: Unit::Ratio, kind: Kind::Rate },
);
```

`stats["core0.ipc"]` just works. `stats.summary()` prints it under the core's
section. Every consumer (Python API, text dump, future JSON export) gets the
same value from the same source. No per-team `parse_stats.py` computing IPC a
slightly different way.

The formula language is deliberately small (`Div`, `Ratio`, `Sum`) — enough for
IPC/CPI/accuracy/miss-rate and nothing more. Anything more elaborate is a
Python problem.

Divide-by-zero returns `0.0`, not `NaN`. This matches the legacy stats
behavior; users comparing runs across configurations don't want `NaN`
poisoning their spreadsheets when a counter is legitimately zero (e.g.,
`bp.accuracy` for a workload with zero branches).

## Auto-generated summary

`stats.summary()` replaces the hand-written `print_sections` function.
Sections come from grouping metadata by subject; alignment, unit formatting,
and derived-metric placement fall out of the metadata. Adding a counter is a
one-line change; the summary picks it up automatically.

The output shape matches the old `SimStats::print_sections` verbatim
(subject headers, sub-groups, aligned values) so downstream users don't have
to relearn the format. What changes is that adding a new counter no longer
requires editing a 350-line function.

## Python compatibility

Phase 3b preserves the Python dict keys — `sim.stats["instructions_retired"]`
still works. `PyStats::to_dict` becomes a tree adapter that reads from
`stats.get(paths::...)` and writes the old flat names. This lets us reshape
the Rust side without breaking notebooks and scripts that already exist.

The 16 counters that were silently reading zero since Phase 2 (cache hits/
misses, MSHR, prefetch dedup, `stalls_mem`, etc.) are removed rather than
preserved as zero. They'll come back in Phase 3c when `impl Handle for Cache`
wires them through properly.

## What this does not solve

Explicitly out of scope for this design, so future contributors don't try to
squeeze them in:

- **Diff view** (`stats.diff(baseline)`) — natural next step, deferred.
- **Windowed view** (`stats.since(cycle)`) — needs a snapshot mechanism the
  hot path doesn't currently pay for.
- **Full arithmetic query language** — Python is a better place for this.
- **JSON/CSV dump** — one afternoon of work once the tree is stable, but not
  needed for the migration.
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
