# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- A multithreaded bundled program, `raytracer_mt`: a scene of spheres
  over a checkered floor, with shadows and a reflection bounce, its rows
  shared out between N threads (one per CPU by default), printed with the
  cycles the render took. On the P550 preset it renders in 1.31 M cycles
  on one hart, 0.70 M on two, 0.37 M on four and 0.24 M on eight.
  Threaded programs live in `examples/programs/threaded` and build for SE
  mode only; `make se-verify BUNDLED=1` compares it with qemu on four
  threads.
- SE mode answers `/sys/devices/system/cpu/online`, `possible` and
  `present` for the simulated harts, as gem5 does, where the host's files
  showed through: `sysconf(_SC_NPROCESSORS_ONLN)` and `get_nprocs()`
  return the hart count, so a program that starts a thread per CPU starts
  one per hart.
- `make se-verify` also compares a static pthreads program with
  `qemu-riscv64`: a contended mutex, a condition variable, a barrier,
  thread-local storage, joins and a timed wait, on five harts.
  `tools/se/verify.py --programs GLOB --harts N` runs threaded programs of
  one's own.
- SE mode runs threads, as gem5's SE mode does: `clone` starts a thread
  on the lowest-numbered free hart, where it stays, one thread per hart
  with no time-slicing, and fails with `EAGAIN` when no hart is free, so
  a static pthreads program runs with `--harts N` or
  `Config(hart_count=N)`. Threads share their process's memory, heap,
  mappings and descriptors; `futex` waits park a hart until a wake or a
  timeout in simulated time; `exit` ends one thread and wakes its joiner,
  `exit_group` and faults end the process; `gettid`, `set_tid_address`
  and `sched_getaffinity` are served; unmapping memory makes the other
  threads drop their translations, as a TLB shootdown does. `kill`,
  `tkill` and `tgkill` reach the process they name, signal 0 only checks
  that it exists, and signals ignored by default do nothing. Checkpoints
  carry the threads.
- SE mode's `madvise` refuses advice Linux 6.6 does not know, as that
  kernel does, so glibc 2.42 guards a thread's stack with `mprotect`
  rather than taking a guard it never got.
- `Simulator.exit_codes` holds one exit code per SE program, in the
  order the programs were given, where it held one per hart: a hart
  without a program no longer has a `None` entry. Checkpoints are format
  version 8.
- GDB watchpoints: `watch`, `rwatch` and `awatch` stop once a load or
  store to the range has retired, as a hardware trigger with "after"
  timing does, and GDB shows the old and new values.
- A GDB remote stub: `python -m rvsim prog.elf --gdb 1234` waits for GDB
  and `Simulator.gdb(port)` serves it from wherever a run is. Breakpoints
  are stops at a PC, so no guest code is patched; a step retires one
  instruction; registers include the floating-point registers and the
  common CSRs by name; memory is read and written through the selected
  hart's translation; each hart is a thread; Ctrl-C interrupts; the
  guest's exit ends the session with its code. A stop drains the
  pipelines as a hardware halt does, so a debugged run takes more cycles
  ([Debugging with GDB](gdb.md)).
- `run_to(pc=...)` and the Session API's `Pc` stop land exactly on the
  address: an out-of-order core no longer retires the instruction there
  in the same cycle as the ones before it, which could carry a run past
  the stop.
- SE mode runs several programs at once, one per hart, as gem5's
  `se.py -n N --cmd "a;b"` does: `Simulator(programs=[Program(...),
  ...])` or `python -m rvsim a.elf --program "b.elf arg"`. Each program
  has its own address space from the shared RAM and sees the memory
  system the configuration shares; the run ends when the last exits, with
  the first non-zero exit code in hart order, and `Simulator.exit_codes`
  holds each hart's. A program's `getpid` is 1, 2, ... by hart.
  Checkpoints carry every program (format version 7).
- More of gem5's m5ops: `m5_rpns` (`rvsim_time_ns()` in `rvsim.h`) gives
  the simulated time in nanoseconds, `m5_sum` adds its arguments,
  `m5_debug_break` and `m5_switch_cpu` stop the host's run, and
  `m5_panic` ends it with exit code 1.
- SE mode answers `riscv_hwprobe` as the 6.6 kernel its `uname` reports
  would, from the hart's ISA string: F/D, C, V, Zba, Zbb and Zbs, and
  whether misaligned accesses are fast. A static glibc program under SE
  now selects the same routines it would under Linux on rvsim, and the
  unimplemented-syscall warning every such program logged at start-up is
  gone.
- The Session API's fast-forward cache keys checkpoints on the simulator
  build as well, so a boot cached under one build is never restored under
  a later one whose guest would have seen a different device tree or ISA.
- `Config(isa=...)` names the whole ISA, multi-letter extensions
  included, and `misa`, the decoder and the device tree follow it: with
  `isa="rv64gc"` a `clz` is an illegal instruction and the tree names no
  Zbb, as on a core without it, where before every extension executed
  whatever the string said. The string is spelt as `-march`, spike's
  `--isa` and `/proc/cpuinfo` spell it, `G` and `B` expand, the vector
  crypto groups `zvkn`, `zvks` and their `c`/`g` forms stand for their
  members, an unknown name is refused, and `Config().isa_string()` gives
  the resolved string (everything, by default) to derive variants from.
  The native config's `pipeline.misa_override` key is gone; the string
  lives under `isa.string` (#77).
- The generated device tree advertises every extension the hart
  implements, not just `misa`'s letters: `riscv,isa` names Zicbom,
  Zicboz, Zicntr, Zicsr, Zifencei, Zihintpause, Zihpm, Zfh, the Zb and
  Zbk families, the Zvb and Zvk families, Zvfh, `zvl<VLEN>b`, Sstc and
  Svadu as the configuration has them, the `riscv,isa-base` and
  `riscv,isa-extensions` pair says the same for kernels that prefer it,
  `riscv,cbom-block-size` and `riscv,cboz-block-size` carry the block
  size Linux needs before it uses the cache-block instructions, and
  `mmu-type` follows `paging_mode_max` instead of always saying Sv39. A
  Linux guest now enables its Zbb string routines and reports the
  extensions through `hwprobe`, so glibc picks the matching routines
  (#76).
- gem5's `readfile` and `writefile` pseudo-operations (`m5_read_file`,
  `m5_write_file`; `rvsim_read_file` and `rvsim_write_file` in
  `rvsim.h`): a guest reads the host's script file into its memory and
  writes a file into the host's output directory, named by
  `Simulator(script=..., outdir=...)`, `Environment(script=, outdir=)`
  or `--script` and `--outdir`. They retire like a syscall in SE mode,
  reach the program's memory through the hart's own translation, and
  return a count or a negated `errno` in `a0`, in a bare-metal image, a
  user program and a Linux guest alike. See
  [Pseudo-operations](pseudo-ops.md#files).
- The Linux image's `rvsim` tool issues the pseudo-instruction instead
  of writing the sim-control device through `/dev/mem`, and gains
  `rvsim readfile` and `rvsim writefile FILE [NAME]`, like gem5's `m5`.
- The sim-control MMIO device and its `sim_control_base` setting are
  gone: the pseudo-instruction is the one way a guest talks to the
  simulator.
- The Linux image sets `kernel.perf_user_access = 2`, so `rdcycle` works
  from user space on Linux 6.6.
- The Linux image's Buildroot configuration lives in
  `software/guest/buildroot.defconfig`, with a kernel fragment,
  `software/guest/linux.fragment`, that states the vector, Zicbom,
  Zicboz and perf options instead of relying on the kernel's default.
- `Simulator.vmem32` and `vmem64` walk the page tables without filling
  the TLBs, so reading guest memory from Python no longer changes what
  the hart's translation caches hold.
- The HTIF device serves the syscall protocol spike's proxy (fesvr) and
  Chipyard's harness speak: a bare-metal program hands the host an
  eight-word block through `tohost`, the host performs the call (the
  file calls, `write` to the console, `getmainvars` for the arguments
  it was run with) and answers through `fromhost`. `make software` now
  also builds every bundled program with an HTIF start-up and libc into
  `software/bin-htif`, so one ELF prints and exits the same way on rvsim,
  spike and the RTL simulators (#205).
- Checkpoints work in SE mode: the process's mappings, break and heap are
  saved with the hart and RAM, and a run restored from one finishes as
  the uninterrupted run does. A checkpoint is refused while the program
  has a host file open, since the descriptor would not come back. The
  HTIF device's `fromhost` is saved too.
- SE mode loads static position-independent executables (`-static-pie`),
  placing one at the address static executables are linked at; only
  dynamically linked programs are refused. `make se-verify` checks a
  static PIE against qemu with the other check programs.
- A [Coming from gem5](from-gem5.md) guide: what each gem5 habit, from
  `se.py` and `O3CPU` parameters to m5ops, checkpoints, CPU switching
  and O3PipeView, maps to in rvsim, and what rvsim does not do.
- A [Validation](validation.md) page: every check rvsim runs against
  spike, qemu, gem5, the Rocket and BOOM RTL and measured hardware, what
  each compares, how often it runs, and where its numbers stand.
- A pipeline trace for Konata: `python -m rvsim prog.elf --pipeline-trace
  out.kanata`, or `Simulator.open_pipeline_trace(path)` around the part
  of a run to look at, writes where every instruction is each cycle in
  the Kanata log Konata displays, on both backends, squashes included.
  The pipeline reports to a `PipelineTrace` trait, so another viewer's
  format is another implementation. See
  [Pipeline trace](pipeline-trace.md).
- Direct mode is gone. It was an rvsim-only way to run a bare ELF:
  identity translation, a preset stack pointer, the FPU switched on at
  reset, `a1` holding the hart count and an `ecall 93` ending the run
  with no trap handler installed. A bare-metal image now starts as it
  does under spike: every hart at the ELF's entry in machine mode with
  its id in `a0` and a device tree describing the system in `a1`, placed
  in the last 64 KiB of RAM; the program's start-up sets its stack and
  switches the FPU on (`software/libc/crt0_htif.s` and `crt0_smp.s` do),
  and it exits through HTIF or a pseudo-operation. The bare-metal libc
  prints through HTIF only, and the multi-core programs take their hart
  count from the device tree and share the HTIF channel under a lock.
  `Config(initial_sp=...)` is gone with it, and the `direct_mode` and
  `initial_sp` keys of the native config.
- Pseudo-operations are an instruction: gem5's m5op (opcode `0x7b`, the
  operation in bits 31:25, arguments in `a0` and `a1`), performed when it
  retires in any mode and at any privilege, so one `rvsim.h` serves a
  bare-metal image, a user program under SE mode and a program inside a
  Linux guest, and a binary instrumented with gem5's libm5 runs
  unchanged: `m5_exit`, `m5_fail`, `m5_reset_stats`, `m5_dump_stats`,
  `m5_dump_reset_stats`, `m5_checkpoint` and the work markers.
  `regions.elf` is a Linux user binary again. See
  [Pseudo-operations](pseudo-ops.md).
- A user program's stdin, stdout and stderr, and the console HTIF's
  syscalls serve, are the simulator's console: `console="quiet"` and
  `"captured"`, `read_console()`, `write_console()` and the Session API's
  console stops apply to them as they do to the UART, instead of the
  output going straight to the host's stdout. Input goes to the UART
  until a program that talks to the host directly is loaded. The console
  answers `TCGETS2` as well as `TCGETS`, so glibc 2.42's `isatty` holds
  and a user program's stdout is line-buffered, as on a terminal.
- The bundled programs and benchmarks are static Linux user binaries
  built with the glibc toolchain and run under SE mode, as a gem5 user
  would run them; `software/bin-htif` keeps the bare-metal build. The
  bare-metal libc now implements a subset of C's stdio and stdlib
  (`fgets`, `fflush`, `exit`, `int printf`) instead of its own API, and
  a user program reads the cycle and instruction counters without
  trapping. `bench.h` and `rvsim.h` moved to `software/include`.
- SE mode runs a static Linux user program without a kernel: the
  simulator loads the ELF, builds its page tables (Sv39 by default,
  `page_mode` selects Sv48 or Sv57) and stack, runs it in user mode and
  serves its syscalls against the host, as gem5's SE mode does, so
  anything built with a Linux toolchain and linked `-static` runs
  unchanged: `python -m rvsim ./prog -- args`, or `Environment(binary,
  args=[...], env=[...])`. A syscall costs the pipeline a flush and
  nothing else; `clock_hz` sets the clock the time calls read. `make
  se-verify` checks the output and exit code of a set of programs, and
  Embench-IoT with `EMBENCH=1`, against `qemu-riscv64`. One hart, static
  non-PIE executables, no threads yet (see [SE mode](se-mode.md)).
- `make rtl-build` builds Chipyard 1.14.0's Rocket and Medium BOOM V4 as
  Verilator simulators and `make rtl-compare RTL_CORE=rocket|boom` lines
  rvsim's retire timing up with theirs instruction by instruction;
  `tools/diag/rtl_probe.py` measures one mechanism at a time on either.
  [Error against Rocket](rtl-rocket.md) and [Error against BOOM](rtl-boom.md)
  record the results and what each remaining difference is (#177).
- `BranchPredictor.Bht` is Rocket's branch history table: a table of one-
  or two-bit counters indexed by the fetch PC with a hash of the global
  history placed above the PC bits, advanced, rebuilt and reset as
  rocket-chip's `BHT` is. The `rocket` preset uses it in place of the
  fixed `GShare`, which indexed two nearby branches to one counter half
  the time and mispredicted `br_random`'s loop branch as often as its
  random one (#222).
- The in-order backend no longer holds a system instruction in issue until
  it is the oldest. A CSR access, ECALL, MRET, WFI, FENCE.I and SFENCE.VMA
  issue in order and take effect when they retire: the CSR is read and
  written at the head of the ROB, where every older write has been
  applied, as Rocket's WB does and as gem5's MinorCPU executes at commit,
  and the squash a CSR access, fence or MRET causes is taken from commit
  rather than from execute (#216). A CSR read's dependent now waits for
  the read to retire. `csr_squash="Never"` needs the out-of-order
  backend's rename hold and is rejected on the in-order one.
- `fetch_decode_latency`, `decode_rename_latency` and `rename_issue_latency`
  set the front end's depth, which was fixed; the defaults keep it. A
  mispredict, a trap and an MRET cost what the configured depth implies,
  so the `rocket` preset, which collapses decode and rename into Rocket's
  ID stage and issues the cycle after it, pays Rocket's three cycles for a
  mispredict rather than nine (#213).
- `csr_squash` and `fence_squash` say which CSR accesses and whether a
  FENCE squash the instructions fetched behind them, as the modelled core's
  decoder has it; the defaults keep gem5's behaviour (every CSR access on
  the in-order backend, none on the out-of-order one). The `rocket` preset
  squashes after writes to the CSRs Rocket's `write_flush` names and never
  after a read; the `boom` preset after every CSR instruction and every
  FENCE, which BOOM commits with `flush_on_commit` (#212).
- FENCE.I's invalidation of the L1I now reaches the fetch buffer, the copy
  of the line the I-cache last returned, so the instructions after a FENCE.I
  refetch through a miss, as Rocket's and BOOM's do (#211). A FENCE.I's
  cost grows by an L1I miss.
- The in-order backend forwards a load's value to a dependent the cycle
  memory2 delivers it, as it already did a unit's result and as Rocket
  bypasses its D-cache response into the next instruction's execute, so a
  dependent load costs the L1D latency plus one cycle rather than two
  (#210). Every in-order cycle count moves.
- Each commit-log line ends its header with `cycle <n>`: the cycle the
  instruction retired in, or the cycle a trap was taken in, so the log
  doubles as a retire trace that lines up against an RTL core's (#177).
- `Simulator.open_commit_log` writes what a reference model needs to replay
  the run: the architectural state the log starts from, each retired
  instruction's privilege mode, destination register, the CSR it wrote (as
  read back), the FP flags it raised and its load or store (virtual and
  physical address, size and value), and each trap taken. A vector
  instruction's line also gives each vector register it wrote, with the bits
  it filled under a tail- or mask-agnostic policy, and each element it loaded
  or stored. An FP destination was logged as an `x` register.
- `make lockstep` (`tests/conformance/lockstep.py`, `make lockstep-smoke`
  for the smoke configs) replays rvsim's commit log on spike one instruction
  at a time for every riscv-test, program, benchmark and RVV test on every
  single-hart pipeline config, and stops at the first instruction whose PC,
  privilege mode, destination, vector registers, CSR write, FP flags or
  memory access differs. Values the ISA leaves to the implementation
  (counters, timers, IDs, device reads, interrupt arrival, WFI wake-up, lost
  reservations, agnostic vector elements) are taken from rvsim's log. The driver is `tools/lockstep/spike_lockstep.cc`, built against the
  pinned spike. `make test-all` runs it; the smoke run does not.
- A `pmpaddr` register kept all 64 bits written to it, so bits 63:54 read
  back as written (#170). On RV64 they read as zero; only bits 53:0
  (physical address bits 55:2) are kept now.
- Debug triggers read `tdata1`'s `mcontrol` fields from the wrong bits:
  M, S and U at 13, 11 and 10 and execute, store and load at 9, 8 and 7,
  where the debug spec puts them at 6, 4 and 3 and 2, 1 and 0 (#171). A
  trigger written by software that follows the spec, such as a debugger's
  breakpoint, came back disarmed. The fields are now where the spec puts
  them.
- Writing `satp` with a MODE the hart does not support, or one above
  `paging_mode_max`, set MODE to Bare and kept the written ASID and PPN
  (#172). The privileged spec says such a write has no effect, and now it
  has none.
- The `vec_stress` example's masked-operations check failed on rvsim
  (#173): it used the mask-agnostic `__riscv_vadd_vv_i64m2_m`, whose
  inactive elements may be all ones, as rvsim makes them, and expected them
  to hold `va`. It now uses the mask-undisturbed form with `va` as the merge
  operand.
- `misa.B` is set (#174): every hart implements Zba, Zbb and Zbs, so B now
  says so, whatever `Config(isa=...)` names, and the device tree's
  `riscv,isa` gains a `b` (`rv64imafdcbv_sstc` by default). `isa` accepts
  `B`.
- A load or store trigger's breakpoint set `mtval` to the instruction's PC
  (#176). The privileged spec gives the faulting virtual address, which for
  a data trigger is the address accessed; it does now, and a CBO's trigger
  reports the address its other faults report.
- Half-precision arithmetic, scalar (Zfh) and vector (Zvfh), raised the
  invalid flag only for a signaling-NaN operand (#178). It now also raises
  it for ∞ − ∞ in an add or subtract, 0 × ∞ in a multiply, and both in a
  fused multiply-add, where 0 × ∞ is invalid even with a quiet-NaN addend.
  `rv64uzfh-p-fadd` failed on this; the riscv-tests runner now runs the
  `rv64uzfh`, `rv64uzba`, `rv64uzbb`, `rv64uzbc` and `rv64uzbs` suites,
  which were built but never run.
- The vector widening FP operations dropped the invalid flag for a
  signaling-NaN operand (#182): `vfwadd`, `vfwsub` and `vfwmul` (and their
  `.w` forms) at SEW=32 and 16, and `vfwredusum`/`vfwredosum` at SEW=32. The
  host conversion of the operand to the wider format raised it before the
  flags were cleared. The widening FMAs worked only because of where the
  compiler placed that conversion. All of them now check their operands.
- RMM (round to nearest, ties to max magnitude) rounded ties to even,
  scalar and vector, because the host FPU has no such mode (#179). An
  inexact RMM result is now rounded again in software from the operation's
  exact value. The software rounding (`exec/compute/fpu/exact.rs`) is
  property-tested against the host in the four modes the host has. The
  half-precision FMAs, scalar and vector, and the Zvfh widening FMA rounded
  twice (to binary64, then to the result format) in every mode, and now
  round once. The Zvfh widening add, subtract and multiply lost the flags
  of their final rounding to binary32 (#180).
- A vector instruction with `vstart` ≥ `vl` updated its destination (#183):
  a reduction with vl=0 wrote `vs1[0]` into `vd[0]`, and element-wise ops
  filled their tail with ones under a tail-agnostic policy. The vector spec
  says no destination element, agnostic or not, is updated then. Now none
  is, except by the instructions that still execute: those writing an `x`
  or `f` register and the whole-register moves.
- On the O3 backend a vector load could read memory from before an older
  vector store whose element addresses resolved after the load ran (#184).
  The load's micro-ops gave up their load-queue slots as they wrote back,
  so the store's ordering check missed them. A vector load now reserves its
  slots at rename, one per micro-op it expects (an element, or a window of
  an aligned unit-stride access), and its micro-ops hold them until it
  commits. A load with more micro-ops than slots (an oversize load, a
  misaligned unit-stride load, a span split at a fault) reuses its slots
  once no older memory access is in flight. Lockstep against spike stopped
  on this in the RVV `vsoxei16` and `vsseg2e32` tests.
- The vector float-to-integer conversions at a 16-bit width, `vfwcvt.x[u].f.v`
  at SEW=16 (f16 to 32-bit integer) and `vfncvt.x[u].f.w` at SEW=16 (f32 to
  16-bit integer) and their `rtz` forms, truncated whatever `frm` said and
  raised NV only for a NaN (#181). They now round in `frm` (the `rtz` forms
  toward zero) and raise NX for an inexact result and NV for one out of
  range, as the 32- and 64-bit conversions do.

## Releases

- [v2.1.0](versions/V2_1_0_CHANGELOG.md) — 2026-10-08 (device loads, FENCE.I coherence, oldest-only issue timing, `Config(isa=...)`)
- [v2.0.1](versions/V2_0_1_CHANGELOG.md) — 2026-10-07 (decode redirects only when the path changes)
- [v2.0.0](versions/V2_0_CHANGELOG.md) — 2026-10-07 (multi-core coherence, vector extension, event-driven memory system, DDR5, stats by path, Linux sessions and benchmarks)
- [v1.2.0](versions/V1_2_CHANGELOG.md) — 2026-03-21 (squash recovery, SC-L-TAGE + ITTAGE, pipeline fixes)
- [v1.1.0](versions/V1_1_CHANGELOG.md) — 2026-03-21 (memory dependence prediction)
- [v1.0.0](versions/V1_CHANGELOG.md) — 2026-03-20 (initial stable release)
