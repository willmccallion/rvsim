# rvsim

**Cycle-level RISC-V 64-bit system simulator** with a composable Python API for architecture research and design-space exploration.

[PyPI](https://pypi.org/project/rvsim/){ .md-button } [Rust Core (crates.io)](https://crates.io/crates/rvsim-core){ .md-button } [GitHub](https://github.com/willmccallion/rvsim){ .md-button }

---

## What is rvsim?

rvsim models a complete RISC-V system **cycle by cycle**: one or more cores,
their caches, a coherence fabric, a memory controller and the SoC's
devices. Functional simulators such as QEMU and spike say what a program
computes; rvsim says how long it takes and why: pipeline stalls, cache and
TLB misses, branch mispredictions, structural hazards, memory ordering and
the traffic between cores.

It has two pluggable core backends:

- **Out-of-order superscalar**: register renaming onto physical registers,
  a CAM-style issue queue with wakeup and select, a reorder buffer, load and
  store queues with store-to-load forwarding and memory-dependence
  prediction, and in-order commit with precise exceptions.
- **In-order**: configurable width, scoreboard operand tracking and
  program-order issue.

Both share the same frontend, memory stages, commit, memory hierarchy and
devices, and the same definition of what every instruction does, so a
difference between them is a difference in timing alone.

!!! note "Accuracy"
    rvsim simulates every cycle, but it is not cycle-accurate to any one
    machine yet. It models how real cores behave and measures itself two
    ways. Against gem5's O3 CPU on a set of single-behaviour kernels, the
    compute- and branch-bound programs are within a few percent; programs
    bound by memory or vector code are still 10 to 60% apart, partly from
    gaps in rvsim's memory and vector models and partly from places where
    gem5 differs from hardware (see [Error against gem5](error.md)).
    Against hardware, the `p550()` and `cortex_a72()` presets reproduce
    their published cache and memory latencies, and the A72 preset runs
    CoreMark within 6% of a Raspberry Pi 4 (see
    [Linux Benchmarks](examples/linux-benchmarks.md)). We are working to
    close the remaining gaps.

### Key facts

- **ISA**: RV64GC with the vector extension (RVV 1.0, VLEN 128 to 2048),
  Zba, Zbb, Zbc, Zbs, Zbkb, Zbkx, Zfh, Zicbom, Zicboz, Zvfh, Zvbb, Zvbc and
  the Zvkn, Zvks and Zvkg vector crypto subsets; M, S and U privilege modes
  with Sv39, Sv48 and Sv57 paging, PMP, Sstc, Svadu and debug triggers
  (see [ISA](architecture/isa.md))
- **Correctness**: passes all 134 `riscv-tests`; the chipsalliance
  `riscv-vector-tests` suite is cross-checked against spike
- **Systems**: boots Linux 6.6 through OpenSBI to a BusyBox shell, on one
  core or several kept coherent by a MESI fabric
- **Speed**: about 0.6 to 0.85 million simulated cycles per second on the
  out-of-order backend and 1.5 to 1.7 million on the in-order backend, on
  one host core (measured on bare-metal programs; Linux runs at 0.25 to 0.5
  million with devices and an 8-hart default)

## Quick Start

```bash
pip install rvsim
```

```python
from rvsim import Config, BranchPredictor, Cache, Environment

config = Config(
    width=4,
    branch_predictor=BranchPredictor.TAGE(),
    l1d=Cache("32KB", ways=8, latency=1, mshr_count=8),
    l2=Cache("256KB", ways=8, latency=10),
)

result = Environment(binary="program.elf", config=config).run()
print(result.stats.query(r"^ipc$|bp\.committed\.accuracy|l1d\.miss_rate"))
```

## Who is this for?

- **Computer architecture students** learning how pipelines, caches, and branch predictors work
- **Researchers** exploring microarchitectural design spaces (cache sizing, predictor comparison, width scaling, core counts and interconnects)
- **RISC-V developers** who need cycle-level visibility into how their code executes
- **Educators** teaching computer architecture with a simulator that boots real software

## What's next?

<div class="grid cards" markdown>

- **[Getting Started](getting-started.md)**: install, run your first simulation, understand the output
- **[Configuration](configuration.md)**: every parameter, with its default: caches, predictors, backends, functional units, memory, multi-core
- **[API Reference](api.md)**: `Config`, `Environment`, `Session`, `Simulator`, `Sweep`, `Stats`
- **[Design](architecture/design.md)**: how the simulator is structured and why
- **[Architecture](architecture/pipeline.md)**: the pipeline, memory hierarchy, branch prediction, ISA, devices and multi-core in depth

</div>
