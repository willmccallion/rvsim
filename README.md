# rvsim

[![PyPI](https://img.shields.io/pypi/v/rvsim)](https://pypi.org/project/rvsim/)
[![crates.io](https://img.shields.io/crates/v/rvsim-core)](https://crates.io/crates/rvsim-core)
[![ISA Tests](https://img.shields.io/badge/riscv--tests-188%2F188-brightgreen)](#isa--privileged-architecture)
[![ISA](https://img.shields.io/badge/ISA-RV64IMAFDCV-blue)](#isa--privileged-architecture)
[![Boots Linux](https://img.shields.io/badge/Linux%206.6-boots%20to%20shell-blue)](#linux-boot)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue)](#license)

Cycle-level RISC-V 64-bit system simulator with a composable Python API for architecture research and design-space exploration.

[Documentation](https://willmccallion.github.io/rvsim/) · [PyPI](https://pypi.org/project/rvsim/) · [Rust Core (crates.io)](https://crates.io/crates/rvsim-core) · [Changelog](https://willmccallion.github.io/rvsim/changelog/)

---

rvsim models a complete superscalar processor cycle by cycle. It implements two pluggable microarchitectural backends — out-of-order and in-order — sharing a common frontend, memory hierarchy, and SoC device layer. It runs static Linux user programs directly in SE mode, boots Linux 6.6 through OpenSBI to a BusyBox shell and passes all 188 [`riscv-tests`](https://github.com/riscv-software-src/riscv-tests). The chipsalliance [`riscv-vector-tests`](https://github.com/chipsalliance/riscv-vector-tests) suite is cross-checked against [spike](https://github.com/riscv-software-src/riscv-isa-sim).

> **Accuracy.** rvsim simulates every cycle, but it is not cycle-accurate to any one machine yet. It models how real cores behave and measures itself two ways: against gem5's O3 CPU, where compute- and branch-bound kernels are within a few percent and memory- and vector-bound ones are still 10 to 60% apart ([Error against gem5](https://willmccallion.github.io/rvsim/error/)), and against hardware, where configurations calibrated to the SiFive P550 and the Cortex-A72 reproduce their measured cache and memory latencies and the A72 one runs CoreMark within 6% of a Raspberry Pi 4 ([Linux Benchmarks](https://willmccallion.github.io/rvsim/examples/linux-benchmarks/)). We are working to close the remaining gaps.

## Install

```bash
pip install rvsim
```

Requires Python 3.11+. Pre-built abi3 wheels cover Linux (x86_64, aarch64), macOS (x86_64, arm64) and Windows (x86_64). SE mode needs a Linux or macOS host; bare-metal and full-system runs work on all three.

## Quick Start

The examples run the bundled programs, which a source checkout builds with `make -C software` (see [Building from Source](#building-from-source)). They are static Linux user binaries, run in [SE mode](#se-mode).

```python
from rvsim import Config, BranchPredictor, Cache, Environment

config = Config(
    width=4,
    branch_predictor=BranchPredictor.Tage(),
    l1d=Cache("32KB", ways=8, latency=1, mshr_count=8),
    l2=Cache("256KB", ways=8, latency=10),
)

result = Environment(binary="software/bin/programs/qsort.elf", config=config).run()
print(result.stats.query(r"^ipc$|bp\.committed\.(accuracy|mispredicts)|(l1d|l2)\.miss_rate"))
```

```
core0.bp.committed.accuracy      0.8731
core0.bp.committed.mispredicts  388,580
core0.cache.l1d.miss_rate        0.0184
core0.cache.l2.miss_rate         0.1581
ipc                              1.5140
```

Stats are keyed by path (`core0.cache.l1d.misses`, `hart0.retired_insts`);
see [Stats & Observability](https://willmccallion.github.io/rvsim/architecture/stats/).

### Command Line

```bash
rvsim software/bin/programs/qsort.elf                  # run an ELF, print stats on exit
rvsim software/bin/programs/qsort.elf --watch          # live dashboard: IPC, cache hit rates, stalls
rvsim software/bin/programs/qsort.elf --preset boom --json stats.json
rvsim examples/analysis/branch_predict.py              # run a script that uses the API
rvsim list                                             # list the bundled programs
rvsim bench coremark dhrystone                         # run benchmarks inside Linux
```

`rvsim` picks the mode from the file: an ELF runs as a program, a `.py` as a script, anything else as a kernel image. `--config FILE` loads a `Config` from a Python file, `--limit 5M` caps the cycles and `--harts N` sets the hart count. `rvsim bench` boots the Linux image from `make linux` once (cached), switches to the configuration under study and measures CoreMark, Dhrystone, Whetstone, STREAM, mbw, `lat_mem_rd` and stress-ng each as its own region. `rvsim --help` and `rvsim bench --help` list the rest.

## Features

### Two Pipeline Backends

**Out-of-order superscalar** — Physical register files with speculative and committed rename maps and branch checkpoints, CAM-style issue queue with wakeup/select and oldest-first priority, stores that issue their address and data separately, reorder buffer for in-order commit with precise exceptions, load queue with store-set memory-dependence prediction, store buffer with forwarding, and a configurable functional unit pool (per-type counts and latencies).

**In-order** — Configurable width, scoreboard-based operand tracking, program-order issue onto the same functional unit pool, backpressure gating. Shares the same frontend and commit/memory/writeback stages as the O3 backend, making both modes directly comparable on identical workloads.

Both backends enforce identical serialization semantics: system/CSR instructions wait for all older completions, FENCE respects predecessor/successor ordering bits, loads wait for older store address resolution.

### Memory Hierarchy

- **SV39 / SV48 / SV57 virtual memory** — separate iTLB/dTLB, an optional shared L2 TLB, a hardware page table walker whose PTE reads go through the L1D, and Svade or Svadu A/D handling
- **L1i / L1d / L2 / L3 caches** — independently configurable size, associativity, latency, and replacement policy (LRU, PLRU, FIFO, Random, MRU)
- **Non-blocking caches at every level** via MSHRs with request coalescing and writeback buffers; caches hold tags only and every access takes effect where it is served
- **Hardware prefetchers** per cache level: next-line, stride, stream, tagged; plus a PC-indexed stride prefetcher in the load/store unit (`LoadPrefetcher.Stride`) and a store-miss stream prefetcher into the L2 (`StorePrefetcher.Stream`)
- **Inclusion policies**: non-inclusive, inclusive (back-invalidation), exclusive (L1-L2 swap)
- **Memory controllers** — fixed latency with a bandwidth limit, a row-buffer DRAM model, or a JEDEC DDR5 controller with command timing, refresh, power-down, ECC (SEC-DED or ChipKill) and patrol scrubbing

### Branch Prediction

Seven pluggable predictors with shared BTB, RAS, and global history register:

| Predictor | Description |
|-----------|-------------|
| Static | Always not-taken (baseline) |
| GShare | PC XOR global history, 2-bit counters |
| BHT | Rocket's branch history table: counters indexed by the PC, with hashed global history above the PC bits |
| Tournament | Local + global two-level adaptive with meta-predictor |
| Perceptron | Neural predictor with weight vectors |
| TAGE | Tagged geometric history lengths |
| ScLTage | TAGE-SC-L (TAGE, loop predictor, statistical corrector) with ITTAGE for indirect targets; defaults to Seznec's 64KB CBP-5 configuration |

RAS recognizes both x1 and x5 as link registers per RISC-V spec Table 2.1, including coroutine swap detection.

### ISA & Privileged Architecture

**RV64IMAFDC + V** — base integer, multiply/divide, atomics (LR/SC + AMO), single/double float with IEEE 754 NaN-boxing, compressed instructions, and the V vector extension (RVV 1.0), with Zicsr, Zifencei, Zicntr, Zihpm, Zihintpause, Zba, Zbb, Zbc, Zbs, Zbkb, Zbkc, Zbkx and Zfh. M/S/U privilege modes, trap delegation, MRET/SRET, WFI, SFENCE.VMA, FENCE/FENCE.I, PMP (16 regions), Sstc, Svadu, and two Sdtrig `mcontrol` exact-address triggers. Cache management ops via Zicbom and Zicboz.

Multi-core systems (`Config(hart_count=N)`) give every hart its own core and private caches behind a MESI coherence fabric: a broadcast or snoop-filter home agent at the LLC and a crossbar, ring, mesh, torus or hypercube interconnect, with per-hart CLINT and PLIC contexts and a device tree that enumerates every hart.

The vector extension supports configurable VLEN (default 256) and ELEN=64. Implemented sub-extensions: Zvfh (half-precision FP), Zvbb / Zvbc (bit-manip and carryless multiply), Zvkn (AES, SHA-256 and SHA-512), Zvks (SM4 and SM3), Zvkg (GHASH). Vector ops are cross-checked against [spike](https://github.com/riscv-software-src/riscv-isa-sim).

Passes all **188/188** tests in [`riscv-software-src/riscv-tests`](https://github.com/riscv-software-src/riscv-tests) and the chipsalliance [`riscv-vector-tests`](https://github.com/chipsalliance/riscv-vector-tests) suite.

### SoC Devices

CLINT timer, PLIC interrupt controller, 16550A UART, VirtIO MMIO block device, Goldfish RTC, SYSCON (poweroff/reboot), HTIF. Auto-generated device tree blob.

## Python API

### Configurations and Presets

`Config()` with no arguments is a desktop-class out-of-order core of the mid-2020s at 4 GHz: 6 wide, a 320-entry ROB, the 64KB TAGE-SC-L, a 48KB L1D, 1MB L2 and 32MB L3 with every prefetcher on, and two channels of DDR5-5600. Every field can be overridden (`Config(width=4)`) or changed on a copy (`config.replace(...)`). The `presets` module adds ready-made machines:

- `presets.basic()`: the default machine, identical to `Config()`
- `presets.fast()`: an Apple M4 P-core class core, 8 wide at 4.4 GHz with a 630-entry ROB
- `presets.rocket()`, `presets.boom()`: Chipyard 1.14's Rocket and Medium BOOM, as their RTL builds them
- `presets.linux(harts=8, core=...)`: a core placed in the multi-core system the bundled Linux image boots on

`basic`, `fast`, `rocket` and `boom` are also the CLI's `--preset` names. See [Configuration](https://willmccallion.github.io/rvsim/configuration/) for every parameter.

### Comparing Configurations

```python
from rvsim import BranchPredictor, Config, Environment, Stats

rows = {}
for name, bp in [("GShare", BranchPredictor.GShare()), ("Tage", BranchPredictor.Tage())]:
    r = Environment("software/bin/programs/maze.elf", Config(branch_predictor=bp)).run()
    rows[name] = r.stats.query(r"^ipc$|bp\.committed\.accuracy")

print(Stats.tabulate(rows, title="Branch Predictor Comparison"))
```

### Parallel Sweeps

`Sweep` distributes all (binary, config) combinations across CPU cores:

```python
from rvsim import Sweep, Config, Cache

results = Sweep(
    binaries=[f"software/bin/programs/{p}.elf" for p in ["qsort", "mandelbrot", "maze"]],
    configs={
        f"L1={s}": Config(l1d=Cache(s, ways=8, mshr_count=8), console="Quiet")
        for s in ["8KB", "16KB", "32KB", "64KB"]
    },
).run(parallel=True)

results.compare(metrics=["ipc", "core0.cache.l1d.misses"], baseline="L1=8KB")
```

### Sessions

`Session` runs one workload in phases: fast-forward to a point of interest, switch to the configuration under study, warm it up, and measure regions. Fast-forwards are cached, so the second run of an experiment restores the Linux boot in seconds:

```python
from rvsim import Backend, Session, presets

s = Session.linux(harts=1)
s.fast_forward(until=Session.LOGIN_SHELL)        # boots once, then restored from the cache
s.switch(presets.linux(harts=1, core=presets.fast().replace(backend=Backend.InOrder())))
s.warm_up(command="coremark 0x0 0x0 0x66 20 7 1 2000")
r = s.measure("coremark 0x0 0x0 0x66 20 7 1 2000")
print(r.cycles, r.instructions, r.ipc)
```

Runs stop at `Cycles`, `Instructions`, `Pc`, `Marker`, `Console`, `Exit` or `When` (combined with `|`), and `Session(binary=...)` runs a bare-metal ELF the same way. See [Linux Boot](https://willmccallion.github.io/rvsim/examples/linux-boot/) and the [API Reference](https://willmccallion.github.io/rvsim/api/#sessions).

### Low-Level Control

```python
from rvsim import Simulator, Config, reg, csr

cpu = Simulator(Config(width=4), binary="software/bin/programs/qsort.elf")

for _ in range(1000):
    cpu.tick()

cpu.run_until(lambda sim: sim.instructions_retired >= 100_000)
cpu.pipeline_snapshot().visualize()

print(hex(cpu.pc), cpu.privilege)
print(hex(cpu.csrs[csr.SATP]))
print(hex(cpu.vmem64[cpu.regs[reg.SP]]))

cpu.save("checkpoint.bin")
cpu.restore("checkpoint.bin")
```

`run_until` also stops at a PC (`pc=`) or a privilege level (`privilege=`). In SE mode PCs and `vmem64` addresses are the program's virtual addresses; `mem64` reads physical memory, which is where a bare-metal image (`software/bin-htif/`, loaded at `0x8000_0000`) lives.

## Analysis Scripts

Ready-to-run design-space exploration in `examples/analysis/`:

| Script | Description |
|--------|-------------|
| `branch_predict.py` | Accuracy comparison across all seven predictors |
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

Requires Rust (the version in `rust-toolchain.toml`), Python 3.11+, and two RISC-V GCCs for the bundled programs: a Linux one (`riscv64-linux-gnu-gcc`; `SE_TARGET=` overrides the prefix) and a bare-metal one (`riscv64-elf-gcc`; `TARGET=`). `nix develop` provides all of them.

```bash
git clone https://github.com/willmccallion/rvsim
cd rvsim
python3 -m venv .venv && source .venv/bin/activate
pip install -r requirements-dev.txt
maturin develop --release
make -C software
```

## SE Mode

SE mode runs a static Linux user program without a kernel, as gem5's SE mode and QEMU's user mode do: rvsim loads the ELF, builds its Sv39 page tables and stack, runs it in user mode and serves its syscalls against the host's files, with time taken from the simulated clock. The bundled programs run this way; anything built with a Linux cross toolchain and linked `-static` or `-static-pie` runs unchanged.

```bash
riscv64-linux-gnu-gcc -O2 -static -march=rv64gc -mabi=lp64d prog.c -o prog.elf
rvsim prog.elf --env LANG=C -- --iterations 100 input.txt
rvsim software/bin/programs/fib.elf --program software/bin/programs/qsort.elf   # one per hart
```

Each thread a program starts takes a hart of its own, so give the run as many harts as it has threads (`--harts N`). `Simulator(programs=[Program(...), ...])` and `Environment(binary=..., args=..., env=...)` do the same from Python, and checkpoints carry every thread. Programs mark regions to measure with gem5's m5op [pseudo-operations](https://willmccallion.github.io/rvsim/pseudo-ops/), which bare-metal images and programs inside the Linux guest use too. See [SE mode](https://willmccallion.github.io/rvsim/se-mode/).

## Linux Boot

Boots Linux 6.6 through OpenSBI to a BusyBox shell. The
default boot is eight out-of-order cores from the `fast` preset (64KB
TAGE-SC-L) with coherent private caches over a mesh and four channels of
DDR5-5600. CI boots it on two harts, logs in and powers off after every merge to `main` and nightly. The root filesystem carries a benchmark suite (CoreMark, Dhrystone, Whetstone, STREAM, lmbench, mbw, stress-ng and others) and an `rvsim` guest tool that marks regions for the stats and exchanges files with the host.

```bash
make linux                          # Build kernel + rootfs via Buildroot
make run-linux                      # Boot 8 SMP cores (login: root, no password)
rvsim tools/boot_linux.py --harts 1   # Single core
```

## Debugging and Validation

- **GDB**: `rvsim prog.elf --gdb 1234` waits for `target remote :1234`; breakpoints, single steps, registers and memory, with each hart a thread. `Simulator.gdb(port)` does the same from Python. See [Debugging with GDB](https://willmccallion.github.io/rvsim/gdb/).
- **Pipeline trace**: `--pipeline-trace FILE` (or `Simulator.open_pipeline_trace`) writes every instruction's stage each cycle as a Kanata log for [Konata](https://github.com/shioyadan/Konata). See [Pipeline trace](https://willmccallion.github.io/rvsim/pipeline-trace/).
- **Lockstep against spike**: `make lockstep` replays every retired instruction on spike and compares its PC, privilege, register and CSR writes and memory accesses.
- **SE mode against qemu**: `make se-verify` checks that programs print the same output and exit with the same code under rvsim and `qemu-riscv64`.
- **Timing references**: `make rtl-compare RTL_CORE=rocket|boom` compares retire timing with Chipyard's Rocket and BOOM RTL under Verilator (the first build takes about an hour, in `nix develop .#rtl`), and `make compare-gem5` compares cycle counts with gem5's O3 CPU. See [Error against Rocket](https://willmccallion.github.io/rvsim/rtl-rocket/), [Error against BOOM](https://willmccallion.github.io/rvsim/rtl-boom/) and [Error against gem5](https://willmccallion.github.io/rvsim/error/).

[Validation](https://willmccallion.github.io/rvsim/validation/) lists every check, how often it runs and where the numbers stand. `make help` lists the targets.

## Documentation

Full documentation including architecture deep-dives, API reference, and examples:

**[willmccallion.github.io/rvsim](https://willmccallion.github.io/rvsim/)**

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the repository layout, the checks a
change must pass, and the policy on AI-assisted contributions.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
