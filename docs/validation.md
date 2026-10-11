# Validation

What rvsim is checked against, how often, and where the numbers stand.
Two kinds of check: that programs run correctly, against spike, qemu and
self-checking suites; and that the cycles are right, against gem5, the
Rocket and BOOM RTL, and measured hardware. The dates say when a number
was last taken; the checks in the first table run on every change and
block a merge when they fail.

## On every change

CI runs these on every pull request and every push to `main`; `make
test-all-smoke` runs the same set locally.

| Check | What it compares | Scope |
|---|---|---|
| Rust tests | The pipeline, caches, devices, ISA and system code against their specifications: unit tests and hand-assembled integration programs on both backends | about 2,460 tests, with and without the `commit-log` feature |
| Python tests | The Python API, the command line, checkpoints, the Session API, SE mode, HTIF, pseudo-ops, the pipeline trace and the bundled programs | about 90 tests |
| [riscv-tests](https://github.com/riscv-software-src/riscv-tests) | Every ISA test (`rv64ui`, `um`, `ua`, `uf`, `ud`, `uc`, `mi`, `si`) ends with its pass code | 134 of 134, on the 16 smoke configurations, the default machine and `presets.inorder()` among them |
| [riscv-vector-tests](https://github.com/chipsalliance/riscv-vector-tests) | A sample of every vector instruction class, cross-checked element by element against spike | the sample on 2 configurations; the full set below |
| Multi-core programs | The bundled self-checking programs (spinlocks, AMO counters, barriers, false sharing, litmus tests) on 2 and 4 harts | every program on the smoke configurations, 4 harts on 4 of them |
| SE mode against qemu-riscv64 | The same static Linux program, run on both, prints the same output and exits with the same code | 6 check programs (one runs pthreads on five harts), 19 Embench-IoT benchmarks and 44 bundled programs, `raytracer_mt` on four threads; the two that take minutes join with `SLOW=1` |
| Lint and format | `clippy` at its pedantic and nursery levels, `rustfmt`, `ruff` | the whole tree |

## On demand and before a release

`make test-all` runs every suite on every one of the 74 pipeline
configurations in `tests/conformance/configs/pipelines.py` (backends,
widths, reorder-buffer and register-file sizes, predictors, functional
unit mixes, cache arrangements). It takes several CPU-hours. `make
prerelease` runs it and boots Linux.

| Check | What it compares | Result |
|---|---|---|
| riscv-tests, all configurations | As above | 134 of 134 on all 74 |
| riscv-vector-tests, full set | 3,023 programs, every element of every vector register and every memory access against spike | 3,023 of 3,023 at VLEN 128 (2026-09-28) |
| Lockstep against spike (`make lockstep`) | Every retired instruction's PC, privilege, destination register, CSR write and memory access, replayed on spike one instruction at a time, over the riscv-tests, the bundled programs, the benchmarks and the vector tests | clean on all configurations (2026-10-10); vector register contents are not yet compared, interrupts are taken from rvsim |
| Multi-core programs | As above, on 2, 4 and 8 harts | clean |
| Linux boot | Linux 6.6 through OpenSBI to a BusyBox shell | boots on one and eight harts |

## Timing

Correct is not the same as right on time. These compare rvsim's cycle
counts with references that have their own timing, each on a matched
configuration.

| Reference | What it compares | Result | Detail |
|---|---|---|---|
| gem5's O3 CPU | Whole-program cycle counts on 32 small kernels built to isolate one behaviour each, on a P550-like 3-wide machine | mean absolute error 10.1%, median 3.3% (2026-10-04) | [Error against gem5](error.md) |
| Rocket (Chipyard RTL) | Retire timing instruction by instruction on 36 programs, against the Verilator simulation of the core | mean absolute error 16.6%, median 9.7% (2026-10-10) | [Error against Rocket](rtl-rocket.md) |
| Medium BOOM V4 (Chipyard RTL) | The same on 35 programs | mean absolute error 19.1%, median 13.2% (2026-10-10) | [Error against BOOM](rtl-boom.md) |
| SiFive P550 (EIC7700X) | Load-to-use latency per cache level, from a dependent pointer chase, and Linux benchmark scores | 3.0 / 13.1 / 39.0 / 267 cycles against 3 / 13 / ~38 / 272 measured | [Linux Benchmarks](examples/linux-benchmarks.md) |
| Arm Cortex-A72 | The same | 4.0 / 20.3 / 242 cycles against 4 / 21 / 243 measured; 5.23 CoreMark/MHz against 5.50 on a Raspberry Pi 4 | [Linux Benchmarks](examples/linux-benchmarks.md) |

gem5 is a reference that can be compared cycle by cycle, not the target:
where it times something the way a simulator does rather than the way a
core does, rvsim follows the core and the difference stays in the error
([decision 12](architecture/decisions/0012-the-model-follows-real-cores.md)).
The RTL comparisons find bugs, not structure to copy: Rocket's and BOOM's
pipelines are simpler than the cores rvsim's presets model, and a
difference that a real, more complex core could also show is not a bug.

## What is not checked

- Vector register contents under lockstep: spike and rvsim legally differ
  on tail- and mask-agnostic elements, so lockstep compares scalar state
  and memory only; the vector suite covers the registers.
- Interrupt timing against spike: lockstep takes interrupts from rvsim.
- Threads against Linux's scheduler: SE mode runs one thread per hart and
  never time-slices, so a program with more threads at once than harts
  cannot run, and a wait costs no kernel time.
- Whole-program cycle counts against hardware: the two presets reproduce
  measured latencies and two benchmark scores, not a traced program.
- The RTL comparison needs an hour-long Verilator build, so it runs on
  demand (`make rtl-build`, `make rtl-compare RTL_CORE=rocket|boom`), not
  in CI.

## Keeping this page honest

Every number here carries the date it was taken. The commands that
produce them are the `make` targets named above; when a change moves a
number, the page moves with it in the same pull request. Timing numbers
are expected to move: a fix that makes rvsim closer to the hardware it
models may move it further from gem5.
