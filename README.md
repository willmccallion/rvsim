# rvsim

[![PyPI](https://img.shields.io/pypi/v/rvsim)](https://pypi.org/project/rvsim/)
[![crates.io](https://img.shields.io/crates/v/rvsim-core)](https://crates.io/crates/rvsim-core)
[![ISA Tests](https://img.shields.io/badge/riscv--tests-134%2F134-brightgreen)](#isa--privileged-architecture)
[![ISA](https://img.shields.io/badge/ISA-RV64IMAFDCV-blue)](#isa--privileged-architecture)
[![Boots Linux](https://img.shields.io/badge/Linux%206.6-boots%20to%20shell-blue)](#linux-boot)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

Cycle-level RISC-V 64-bit system simulator with a composable Python API for architecture research and design-space exploration.

[Documentation](https://willmccallion.github.io/rvsim/) · [PyPI](https://pypi.org/project/rvsim/) · [Rust Core (crates.io)](https://crates.io/crates/rvsim-core) · [Changelog](https://willmccallion.github.io/rvsim/changelog/)

---

rvsim models a complete superscalar processor cycle by cycle. It implements two pluggable microarchitectural backends — out-of-order and in-order — sharing a common frontend, memory hierarchy, and SoC device layer. It boots Linux 6.6 through OpenSBI to a BusyBox shell and passes all 134/134 [`riscv-tests`](https://github.com/riscv-software-src/riscv-tests). The chipsalliance [`riscv-vector-tests`](https://github.com/chipsalliance/riscv-vector-tests) suite is cross-checked against [spike](https://github.com/riscv-software-src/riscv-isa-sim).

> **Accuracy.** rvsim simulates every cycle, but it is not yet cycle-accurate: its timing model follows gem5's O3 CPU, and on our microbenchmarks its cycle counts are still tens of percent away from gem5's (see [Error against gem5](https://willmccallion.github.io/rvsim/error/)). We are working to close that gap.

## Install

```bash
pip install rvsim
```

Requires Python 3.10+. Ships pre-built wheels for Linux x86_64.

## Quick Start

```python
from rvsim import Config, BranchPredictor, Cache, Environment

config = Config(
    width=4,
    branch_predictor=BranchPredictor.TAGE(),
    l1d=Cache("32KB", ways=8, latency=1, mshr_count=8),
    l2=Cache("256KB", ways=8, latency=10),
)

result = Environment(binary="software/bin/programs/qsort.elf", config=config).run()
print(result.stats.query(r"^ipc$|bp\.committed\.(accuracy|mispredicts)|(l1d|l2)\.miss_rate"))
```

```
core0.bp.committed.accuracy      0.8750
core0.bp.committed.mispredicts  386,904
core0.cache.l1d.miss_rate        0.0914
core0.cache.l2.miss_rate         0.5149
ipc                              1.1588
```

Stats are keyed by path (`core0.cache.l1d.misses`, `hart0.retired_insts`);
see [Stats & Observability](https://willmccallion.github.io/rvsim/architecture/stats/).

## Features

### Two Pipeline Backends

**Out-of-order superscalar** — Physical register file with dual rename maps (speculative + committed), CAM-style issue queue with wakeup/select and oldest-first priority, reorder buffer for in-order commit with precise exceptions, load queue for memory ordering violation detection, store buffer with forwarding, and a configurable functional unit pool (per-type counts and latencies).

**In-order** — Configurable width, scoreboard-based operand tracking, FIFO issue queue with head-of-queue blocking, backpressure gating. Shares the same frontend and commit/memory/writeback stages as the O3 backend, making both modes directly comparable on identical workloads.

Both backends enforce identical serialization semantics: system/CSR instructions wait for all older completions, FENCE respects predecessor/successor ordering bits, loads wait for older store address resolution.

### Memory Hierarchy

- **SV39 / SV48 / SV57 virtual memory** — separate iTLB/dTLB, shared L2 TLB, full hardware page table walker with A/D bit management
- **L1i / L1d / L2 / L3 caches** — independently configurable size, associativity, latency, and replacement policy (LRU, PLRU, FIFO, Random, MRU)
- **Non-blocking L1D** via MSHRs with request coalescing
- **Hardware prefetchers** per cache level: next-line, stride, stream, tagged
- **Inclusion policies**: non-inclusive, inclusive (back-invalidation), exclusive (L1-L2 swap)
- **Memory controllers** — fixed latency, a row-buffer DRAM model, or a JEDEC DDR5 controller with command timing, refresh and power-down

### Branch Prediction

Six pluggable predictors with shared BTB, RAS, and global history register:

| Predictor | Description |
|-----------|-------------|
| Static | Always not-taken (baseline) |
| GShare | PC XOR global history, 2-bit counters |
| Tournament | Local + global two-level adaptive with meta-predictor |
| Perceptron | Neural predictor with weight vectors |
| TAGE | Tagged geometric history lengths |
| ScLTage | TAGE-SC-L (TAGE, loop predictor, statistical corrector) with ITTAGE for indirect targets; defaults to Seznec's 64KB CBP-5 configuration |

RAS recognizes both x1 and x5 as link registers per RISC-V spec Table 2.1, including coroutine swap detection.

### ISA & Privileged Architecture

**RV64IMAFDC + V** — base integer, multiply/divide, atomics (LR/SC + AMO), single/double float with IEEE 754 NaN-boxing, compressed instructions, and the V vector extension (RVV 1.0). M/S/U privilege modes, full CSR set, trap delegation, MRET/SRET, WFI, SFENCE.VMA, FENCE/FENCE.I, PMP (16 regions). Cache management ops via Zicbom and Zicboz.

Multi-core systems (`Config(hart_count=N)`) give every hart its own core and private caches behind a MESI coherence fabric: a broadcast or snoop-filter home agent at the LLC and a crossbar, ring, mesh, torus or hypercube interconnect, with per-hart CLINT and PLIC contexts and a device tree that enumerates every hart.

The vector extension supports configurable VLEN (default 128) and ELEN=64. Implemented sub-extensions: Zvfh (half-precision FP), Zvbb / Zvbc (bit-manip and carryless multiply), Zvkn (AES + SHA-256), Zvks (SM4), Zvkg (GHASH). Vector ops are cross-checked against [spike](https://github.com/riscv-software-src/riscv-isa-sim).

Passes all **134/134** tests in [`riscv-software-src/riscv-tests`](https://github.com/riscv-software-src/riscv-tests) and the chipsalliance [`riscv-vector-tests`](https://github.com/chipsalliance/riscv-vector-tests) suite. [`riscv-arch-test`](https://github.com/riscv-non-isa/riscv-arch-test) runs through the [RISCOF](https://github.com/riscv-software-src/riscof) framework under `tests/conformance/riscof/`.

### SoC Devices

CLINT timer, PLIC interrupt controller, 16550A UART, VirtIO MMIO block device, Goldfish RTC, SYSCON (poweroff/reboot), HTIF. Auto-generated device tree blob.

## Python API

### Comparing Configurations

```python
from rvsim import BranchPredictor, Config, Environment, Stats

rows = {}
for name, bp in [("GShare", BranchPredictor.GShare()), ("TAGE", BranchPredictor.TAGE())]:
    r = Environment("program.elf", Config(branch_predictor=bp)).run()
    rows[name] = r.stats

print(Stats.tabulate(rows, title="Branch Predictor Comparison"))
```

### Parallel Sweeps

`Sweep` distributes all (binary, config) combinations across CPU cores:

```python
from rvsim import Sweep, Config, Cache

results = Sweep(
    binaries=["qsort.elf", "mandelbrot.elf", "maze.elf"],
    configs={
        f"L1={s}": Config(l1d=Cache(s, ways=8, mshr_count=8), uart_quiet=True)
        for s in ["8KB", "16KB", "32KB", "64KB"]
    },
).run(parallel=True)

results.compare(metrics=["ipc", "core0.cache.l1d.misses"], baseline="L1=8KB")
```

### Low-Level Control

```python
from rvsim import Simulator, Config, reg, csr

cpu = Simulator(Config(width=4), binary="program.elf")

for _ in range(1000):
    cpu.tick()
    cpu.pipeline_snapshot().visualize()

cpu.run_until(pc=0x80001234)
cpu.run_until(privilege="U")

print(hex(cpu.regs[reg.A0]))
print(hex(cpu.csrs[csr.MSTATUS]))
print(cpu.mem64[0x80001000])

cpu.save("checkpoint.bin")
```

## Analysis Scripts

Ready-to-run design-space exploration in `examples/analysis/`:

| Script | Description |
|--------|-------------|
| `branch_predict.py` | Accuracy comparison across all six predictors |
| `cache_sweep.py` | L1D size vs miss rate and IPC impact |
| `design_space.py` | Multi-dimensional width x cache size sweep |
| `o3_inorder.py` | Out-of-order vs in-order backend comparison |
| `width_scaling.py` | IPC vs superscalar width |
| `stall_breakdown.py` | Stall cycles by cause |
| `top_down.py` | Top-down microarchitecture analysis |
| `inst_mix.py` | Instruction class breakdown |

```bash
rvsim examples/analysis/branch_predict.py
rvsim examples/analysis/cache_sweep.py --sizes 4KB 8KB 16KB 32KB 64KB
rvsim examples/analysis/o3_inorder.py --widths 1 2 4
```

## Building from Source

Requires Rust (the version in `rust-toolchain.toml`), Python 3.10+, and a bare-metal RISC-V GCC (`riscv64-elf-gcc` by default; `TARGET=` overrides the prefix). `nix develop` provides all three.

```bash
git clone https://github.com/willmccallion/rvsim
cd rvsim
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements-dev.txt
maturin develop --release
make -C software
```

## Linux Boot

Boots Linux 6.6 through OpenSBI to a BusyBox shell on both backends. The
default boot is eight out-of-order cores from the `fast` preset (64KB
TAGE-SC-L) with coherent private caches over a mesh and four channels of
DDR5-5600.

```bash
make -C software linux              # Build kernel + rootfs via Buildroot
make run-linux                      # Boot 8 SMP cores (login: root, no password)
rvsim tools/boot_linux.py --harts 1   # Single core
```

## Documentation

Full documentation including architecture deep-dives, API reference, and examples:

**[willmccallion.github.io/rvsim](https://willmccallion.github.io/rvsim/)**

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the repository layout, the checks a
change must pass, and the policy on AI-assisted contributions.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
