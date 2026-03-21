# rvsim

**Cycle-accurate RISC-V 64-bit system simulator** with a composable Python API for architecture research and design-space exploration.

[PyPI](https://pypi.org/project/rvsim/){ .md-button } [Rust Core (crates.io)](https://crates.io/crates/rvsim-core){ .md-button } [GitHub](https://github.com/willmccallion/rvsim){ .md-button }

---

## What is rvsim?

rvsim is a hardware-level RV64IMAFDC system simulator that models a complete SoC at **cycle granularity**. Unlike functional simulators (QEMU, Spike) that only care about correctness, rvsim models the microarchitectural details that determine *how fast* a program runs: pipeline stages, cache miss penalties, branch misprediction bubbles, structural hazards, and memory ordering.

It implements two pluggable microarchitectural backends:

- **Out-of-order superscalar** — register renaming, speculative execution, out-of-order issue and commit
- **In-order scalar** — simple scoreboard, blocking issue, deterministic pipeline

Both backends share the same frontend, memory hierarchy, and SoC device layer, making them directly comparable on identical workloads.

### Key facts

- **ISA**: RV64IMAFDC with full M/S/U privileged architecture
- **Correctness**: passes all 134/134 `riscv-tests`
- **Linux boot**: boots Linux 6.6 through OpenSBI to a BusyBox shell on both backends
- **Performance**: ~0.8 MHz simulated clock (wall-clock simulation speed depends on workload and host)

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
print(result.stats.query("ipc|branch|miss"))
```

## Who is this for?

- **Computer architecture students** learning how pipelines, caches, and branch predictors work
- **Researchers** exploring microarchitectural design spaces (cache sizing, predictor comparison, width scaling)
- **RISC-V developers** who need cycle-level visibility into how their code executes
- **Educators** teaching computer architecture with a simulator that boots real software

## What's next?

<div class="grid cards" markdown>

- **[Getting Started](getting-started.md)** — Install, run your first simulation, understand the output
- **[Configuration](configuration.md)** — Every parameter explained: caches, predictors, backends, FU pools
- **[API Reference](api.md)** — Complete reference for `Config`, `Environment`, `Simulator`, `Sweep`, `Stats`
- **[Architecture](architecture/pipeline.md)** — Deep-dive into the pipeline, memory hierarchy, and branch prediction

</div>
