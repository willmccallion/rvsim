# Debugging with GDB

rvsim serves GDB's remote protocol, so a GDB that knows RISC-V
(`gdb-multiarch` on Debian and Ubuntu, `riscv64-unknown-elf-gdb` from a
toolchain, or any GDB built with `--enable-targets=all`) attaches to a run
as it would to a board over JTAG, or to gem5 through `--remote-gdb-port`:
breakpoints, single steps, registers and memory, with each hart a thread.

## Attaching

From the command line, `--gdb PORT` waits for GDB before the run starts:

```bash
python -m rvsim software/bin/programs/qsort.elf --gdb 1234
```

and in another terminal:

```
$ gdb-multiarch software/bin/programs/qsort.elf
(gdb) target remote :1234
(gdb) break main
(gdb) continue
```

From Python, `Simulator.gdb(port)` serves GDB from wherever the run is, so
a script can get past the start-up first:

```python
from rvsim import Config, Simulator

cpu = Simulator(Config(), binary="software/bin/programs/qsort.elf")
cpu.run(limit=100_000)
outcome, code = cpu.gdb(1234)
```

`gdb` returns `("exit", code)` when the guest exits under the debugger,
`("detached", None)` when GDB detaches (`detach`, or `quit`), and
`("killed", None)` after `kill`; a detached run goes on with `run` or
`run_to` as before. From the command line a detached run goes on to its
exit as if GDB had never been there.

A session with GDB 17 on the bundled `qsort` program:

```
(gdb) target remote :1234
0x0000000000010440 in _start ()
(gdb) break rand_next
Breakpoint 1 at 0x1038e: rand_next. (2 locations)
(gdb) continue
Breakpoint 1.1, rand_next () at ../examples/programs/qsort.c:10
10        seed = seed * 6364136223846793005ULL + 1;
(gdb) bt
#0  rand_next () at ../examples/programs/qsort.c:10
#1  main () at ../examples/programs/qsort.c:59
(gdb) finish
Run till exit from #0  rand_next () at ../examples/programs/qsort.c:10
main () at ../examples/programs/qsort.c:59
59          arr[i] = rand_next() % 10000;
(gdb) next
58        for (int i = 0; i < SIZE; i++) {
(gdb) p $cycle
$1 = 182485
(gdb) p/x $satp
$2 = 0x8000000000080000
(gdb) info threads
  Id   Target Id         Frame
* 1    Thread 1 (hart 0) main () at ../examples/programs/qsort.c:58
(gdb) continue
[Inferior 1 (Remote target) exited normally]
```

## What GDB sees

| In GDB | In rvsim |
|---|---|
| `break`, `hbreak`, `tbreak` | a stop when the hart's next instruction to retire is at the address. No guest code is patched, so breakpoints work in ROM, in a kernel before paging is on, and across address spaces, and cost nothing while the run goes on. |
| `stepi`, `nexti`, `step`, `next`, `finish`, `until` | one retired instruction, or one trap taken, on the selected hart; the other harts run for the same cycles |
| `watch`, `rwatch`, `awatch` | a stop once a load or store to the range has retired, with the PC at the next instruction, so GDB shows the old and new values; scalar accesses only |
| `continue`, Ctrl-C | the run at full speed until a breakpoint, the guest's exit, an `m5_debug_break` in the program, or Ctrl-C |
| `info registers`, `p $sp`, `set $a0 = 1` | the hart's integer registers and `pc` |
| `p $f0`, `p $fcsr` | the floating-point registers, `fflags`, `frm` and `fcsr` |
| `p/x $mstatus`, `p/x $satp`, `set $mepc = ...` | the control registers listed below |
| `x/8gx $sp`, `set {int}addr = 1`, `load` | memory through the selected hart's current translation: physical addresses in M-mode, virtual ones under paging, as that hart's own load or store would see them |
| `info threads`, `thread 2` | the harts; thread *n* is hart *n* − 1 |
| the program's exit | the end of the session, with the exit code |

The control registers GDB can name: `fflags`, `frm`, `fcsr`, `sstatus`,
`sie`, `stvec`, `sscratch`, `sepc`, `scause`, `stval`, `sip`, `satp`,
`mstatus`, `misa`, `medeleg`, `mideleg`, `mie`, `mtvec`, `mscratch`,
`mepc`, `mcause`, `mtval`, `mip`, `cycle`, `time`, `instret` and
`mhartid`. Any other CSR is read from Python through a hart's `csrs`.

## A stop costs cycles

A stop is a halt as a hardware debug module makes one: every pipeline
drops what it has in flight, committed stores finish writing to memory,
and fetch restarts at the architectural PC when the run goes on. What GDB
reads is therefore exactly the architectural state, and a run under GDB
takes more cycles than the same run alone: each stop costs the cycles its
stores take to finish and a pipeline refill, and a `step` is a stop per
instruction. Take the numbers to report from a run without a debugger.

## Not covered

- Watchpoints on vector loads and stores: only scalar accesses are
  watched.
- The vector registers.
- Running one hart alone (`set scheduler-locking on`): every resume runs
  every hart.
- Reverse execution, tracepoints, and starting the program from GDB
  (`run`, `start`): the program is loaded already; `continue` runs it.
