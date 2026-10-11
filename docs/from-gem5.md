# Coming from gem5

rvsim does the two things gem5 users do, run a Linux program without a
kernel and boot a kernel, with the same instrumentation habits: m5ops in
the program, checkpoints, stats dumped around a region, a pipeline
viewer. It is RISC-V only, every run is cycle-level (there is no atomic
or timing-simple CPU to drop into), the core is Rust, and everything is
configured and driven from Python. The machines it models are real cores
rather than a generic CPU, and its cycle counts are checked against the
Rocket and BOOM RTL and against measured hardware as well as against
gem5 itself; [Validation](validation.md) has the numbers.

This page maps what you do in gem5 to what you do here.

## Running a program

| gem5 | rvsim |
|---|---|
| `se.py --cmd ./prog --options "a b"` | `python -m rvsim ./prog -- a b` |
| `Process(cmd=[...], env=[...])` in a script | `Environment(binary="./prog", args=["./prog", "a", "b"], env=["K=V"]).run()` |
| `fs.py --kernel ... --disk-image ...` | `make run-linux`, or `Simulator(presets.linux(), kernel="...")` |
| `--num-cpus=N` | `Config(hart_count=N)` |
| `se.py -n 4 --cmd ./threaded` | `python -m rvsim ./threaded --harts 4`, or `Simulator(Config(hart_count=4), binary="./threaded")`; each thread takes a free hart and keeps it, and `clone` fails with `EAGAIN` when none is left, as in gem5 ([SE mode](se-mode.md#threads)) |
| `se.py -n 4 --cmd "a;b;c;d" --options "x;y"` | `Simulator(Config(hart_count=4), programs=[Program("a", ["x"]), Program("b", ["y"]), ...])`, or `python -m rvsim a x --program "b y"`; the run ends when the last program exits ([SE mode](se-mode.md#several-programs-at-once)) |

SE mode takes a static Linux binary, position-independent or not, as
gem5's does, and serves its syscalls in the simulator. Three differences
matter:

- The program runs under real page tables (Sv39 by default; Sv48 and Sv57
  are a setting), so TLB misses and page walks have their full timing.
  gem5's SE mode maps pages without an MMU cost. Every page is mapped at
  load, `brk` and `mmap`, so there are no page faults to serve and a run
  is deterministic.
- A syscall costs the pipeline a serialising instruction, not a kernel's
  worth of work. OS-heavy workloads belong in full-system mode on both
  simulators.
- Threads run as in gem5's SE mode, one per hart with no time-slicing,
  but `clone` makes threads only: `fork` returns `ENOSYS`.

The program's stdin, stdout and stderr are the simulator's console, as
they would be for a kernel's serial port: `Config(console="quiet")`
drops the output, `"captured"` keeps it for `read_console()`.
[SE mode](se-mode.md) has the details and the list of syscalls served.

Full-system mode boots OpenSBI and Linux from a generated device tree,
on one to eight harts, with a UART, CLINT, PLIC, virtio disk and RTC.
The bundled image gives a BusyBox shell; the Session API's
`fast_forward(until=Session.LOGIN_SHELL)` is gem5's "checkpoint after
boot", taken once and cached.

A third kind of program, which gem5 does not have, is the bare-metal
image that prints and exits through HTIF, the protocol spike's proxy and
the Chipyard RTL harness speak. One such ELF runs identically on rvsim,
spike and the RTL, which is how the simulator is cross-checked.

## Configuring the machine

gem5 builds a tree of `SimObject`s in a Python script. rvsim's `Config`
is a flat object of components, each a class with named variants, and
`replace()` derives one configuration from another. A preset is a
complete machine, the way `--cpu-type` plus a cache script is in gem5:
`Config()` itself, a desktop out-of-order core with three cache levels,
every prefetcher and DDR5; `presets.inorder()`, the same machine around a
dual-issue in-order core; `presets.rocket()` and `presets.boom()` as
Chipyard's RTL builds them; and `presets.linux()` for the bundled image.
Where gem5's stdlib composes a board from a processor, a cache hierarchy
and a memory, rvsim starts from the whole default machine and a
configuration names only what differs: `Config(l3=None)`,
`presets.inorder().replace(branch_predictor=BranchPredictor.GShare())`.

| gem5 | rvsim |
|---|---|
| `O3CPU` / `DerivO3CPU` | `Backend.OutOfOrder(...)` |
| `MinorCPU` | `Backend.InOrder()` |
| `AtomicSimpleCPU`, `TimingSimpleCPU` | none: every run is cycle-level |
| `fetchWidth`, `decodeWidth`, `issueWidth`, `commitWidth` | `Config(width=N)`, one width through the pipeline; `load_ports` and `store_ports` for the memory ports |
| `numROBEntries`, `numIQEntries`, `LQEntries`, `SQEntries` | `rob_size`, `issue_queue_size`, `load_queue_size`, `store_buffer_size` |
| `numPhysIntRegs`, `numPhysFloatRegs` | `prf_gpr_size`, `prf_fpr_size` |
| `fuPool` of `FUDesc` | `Fu([...])` |
| `TAGE_SC_L_64KB`, `LTAGE`, `TournamentBP`, `BiModeBP`, `MultiperspectivePerceptron` | `BranchPredictor.ScLTage()`, `.TAGE()`, `.Tournament()`, `.GShare()`, `.Perceptron()`, `.Bht()`, `.Static()` |
| `BTBEntries`, `RASSize` | `btb_size`, `ras_size` |
| store sets (`numStoreSets`, `SSITSize`) | `MemDepPredictor.StoreSet()`, or `.Blind()` |
| `BaseCache(size, assoc, tag_latency, mshrs, prefetcher=StridePrefetcher())` | `Cache("32KB", ways=8, latency=1, mshr_count=8, prefetcher=Prefetcher.Stride())` |
| `--l2cache`, an L3 | `l2=Cache(...)`, `l3=Cache(...)` |
| `LRURP`, `RandomRP`, `TreePLRURP`, `FIFORP`, `MRURP` | `ReplacementPolicy.LRU()`, `.Random()`, `.PLRU()`, `.FIFO()`, `.MRU()` |
| `clusivity` | `Cache.NINE()`, `.Inclusive()`, `.Exclusive()` |
| `StridePrefetcher`, `TaggedPrefetcher`, stream prefetchers | `Prefetcher.Stride()`, `.Tagged()`, `.Stream()`, `.NextLine()`; `LoadPrefetcher.Stride()` and `StorePrefetcher.Stream()` on the load/store unit |
| `DDR4_2400_8x8` and the other `DRAMInterface`s, DRAMsim3 | `MemoryController.DDR5(...)` (channels, ranks, banks, timings, refresh, ECC), `.DRAM()`, `.Simple()` |
| Ruby protocols, directories, `--network=simple|garnet` | the coherence fabric: `HomeAgent.SnoopFilter()` or `.Broadcast()`, `Interconnect.Crossbar()`, `.Ring()`, `.Mesh()`, `.Torus()`, `.Hypercube()` |
| `clk_domain`, `--cpu-clock` | `cpu_clock_mhz` (devices, DRAM); `clock_hz` (the clock SE mode's time syscalls read) |
| `RiscvISA` with `enable_rvv`, `enable_Zcd`, `vlen`, `elen` | `Config(isa="rv64gcv_zba_zbb_zbs_zicbom")`, the ISA string as `-march` spells it, which `misa`, the decoder and the device tree all follow; `Config(vlen=...)` |

The coherence protocol is MESI with a home agent, not a SLICC
state machine you edit. [Configuration](configuration.md) lists every
parameter with its default.

## Running, stopping, measuring

| gem5 | rvsim |
|---|---|
| `m5.simulate()` returning an exit event | `Simulator.run()` to the exit, or `run_to` with stop conditions |
| `m5.simulate(ticks)` | `run(limit=cycles)` |
| exit events from `m5_exit`, work items, checkpoints | `Session.run(Marker(label))`, `Cycles(n)`, `Instructions(n)`, `Pc(addr)`, `Console(pattern)`, `Exit()`, `When(predicate)`, `AnyOf(...)` |
| `m5.stats.dump()`, `m5.stats.reset()` from the script | `Session.measure(until=...)` for a region; `sim.stats_dumps()` for the dumps the program asked for |
| `m5_dump_stats`, `m5_reset_stats`, `m5_exit`, `m5_fail`, work markers in the program | the same instructions; `software/include/rvsim.h` wraps them, and a binary linked with `libm5` runs unchanged ([Pseudo-operations](pseudo-ops.md)) |
| `stats.txt` | `result.stats`, a dict keyed by path (`core0.cache.l1d.misses`, `hart0.retired_insts`, `ipc`) with `query(regex)` and `compare(other)`; `--json FILE` from the command line |
| `--debug-flags=Exec` | `Config(trace=True)` with `RUST_LOG=rvsim::commit=trace`, or the commit log (`open_commit_log`) a lockstep checker replays on spike |
| `--debug-flags=O3PipeView` and `o3-pipeview.py` | `--pipeline-trace FILE`, a Kanata log Konata opens ([Pipeline trace](pipeline-trace.md)) |
| `--remote-gdb-port=N`, `wait_for_remote_gdb`, `m5 debugbreak` | `--gdb N`, or `Simulator.gdb(N)` from a script; each hart is a thread to GDB, and `m5_debug_break` stops in it ([Debugging with GDB](gdb.md)) |

## Checkpoints and switching CPUs

| gem5 | rvsim |
|---|---|
| `m5 checkpoint`, `--checkpoint-restore=N` | `sim.save(path)` and `sim.restore(path)`; the program's `m5_checkpoint` stops the host's run for it to save |
| `--fast-forward`, `--switch-cpu`, `--restore-with-cpu` | `Session.fast_forward(until=...)` reaches a point on a fast configuration (cached by configuration and history), then `switch(config)` continues on the one under study |
| running variants from one checkpoint | `Session.fork({"a": config_a, "b": config_b})` |
| SimPoints | none; stop at a marker, an instruction count or a PC instead |

A checkpoint holds the architectural state, RAM, every device's registers
and, in SE mode, the process; caches, TLBs and predictors start cold on a
restore, as gem5's do, and a checkpoint restores into any configuration
with the same harts, RAM and VLEN.

## What rvsim does not do

- Other ISAs. RISC-V RV64GCV with the bit-manipulation, crypto and
  half-precision extensions, and nothing else.
- Ruby and SLICC: the coherence protocol is fixed, its home agent and
  interconnect are the choices.
- KVM or any native acceleration; a fast-forward runs on a lighter
  configuration, still cycle by cycle.
- Threads in SE mode, and dynamically linked executables.
- GPUs, PCI, network devices, SystemC, Garnet, DRAMsim3.

## What it does that gem5 does not

- Models named cores calibrated on their measurements, and reports how
  far off it is from their RTL and hardware ([Validation](validation.md)).
- Runs every program in lockstep with spike and the vector suite element
  by element against it, so a semantic bug cannot hide behind a timing
  one.
- Bare-metal programs that also run on spike and the Chipyard RTL.
- A pipeline trace for Konata, SE-mode checkpoints and real paging in SE
  mode, built in.

## Five commands to start

```bash
pip install rvsim                                       # or make python in a checkout
python -m rvsim software/bin/programs/qsort.elf         # SE mode, default machine
python -m rvsim software/bin/programs/qsort.elf --preset boom --json qsort.json
python -m rvsim software/bin/programs/qsort.elf --pipeline-trace qsort.kanata
make run-linux                                          # boot the bundled image
```

```python
from rvsim import Environment, presets

result = Environment(binary="software/bin/programs/qsort.elf", config=presets.boom()).run()
print(result.stats.query(r"^ipc$|bp\.committed\.accuracy|l1d\.miss_rate"))
```
