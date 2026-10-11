# Pseudo-operations

A program running on rvsim can ask the simulator for things no hardware
does: reset the statistics, take a labelled snapshot of them, stop the
host's run at a marker, end the simulation with an exit code, or move a
file between its memory and the host. It asks with a pseudo-instruction,
the one gem5's m5ops use, so the same source serves a bare-metal image, a
Linux user program under [SE mode](se-mode.md) and a program inside a
Linux guest, and a binary instrumented with gem5's `libm5` runs on rvsim
unchanged.

## The instruction

The m5op instruction uses the `custom-3` major opcode, `0x7b`, with the
operation number in bits 31:25 and the other fields zero:

```
31        25 24                    7 6       0
+-----------+-----------------------+---------+
| operation |           0           | 1111011 |
+-----------+-----------------------+---------+
```

Its arguments are in `a0` to `a3`, as gem5's are, and the two operations
that return a value leave it in `a0`. The instruction is legal at every
privilege level, costs one issue slot, retires non-speculatively like a
system instruction, and its effect lands at the end of the cycle it
retires. Spike and real hardware trap on it as an illegal instruction, so
a program meant to run there too should keep its pseudo-operations behind
a build flag.

`software/include/rvsim.h` emits it from C:

```c
#include "rvsim.h"

rvsim_reset_stats();       // the stats window starts here
work();
rvsim_dump_stats(1);       // a snapshot labelled 1
rvsim_break(2);            // the host's run_to stops here, label 2
rvsim_exit(0);             // the simulation ends with exit code 0
```

The macro behind them, `RVSIM_PSEUDO_OP(op, a0, a1)`, emits any
operation number.

## The operations

The numbers are gem5's, from `include/gem5/asm/generic/m5ops.h`.

| Operation | gem5 name | `rvsim.h` | What rvsim does |
|---|---|---|---|
| `0x07` | `m5_rpns()` | `rvsim_time_ns()` | `a0` gets the simulated time in nanoseconds, by the clock SE mode's time syscalls read (`clock_hz`). |
| `0x21` | `m5_exit(delay)` | | Ends the run with exit code 0. |
| `0x22` | `m5_fail(delay, code)` | `rvsim_exit(code)` | Ends the run with exit code `a1`. |
| `0x23` | `m5_sum(a, b, c, d, e, f)` | | `a0` gets the sum of `a0` to `a5`. |
| `0x40` | `m5_reset_stats(delay, period)` | `rvsim_reset_stats()` | Zeroes the stats; the window measured from here. |
| `0x41` | `m5_dump_stats(delay, period)` | `rvsim_dump_stats(label)` | Keeps a snapshot of the stats labelled `a0`. |
| `0x42` | `m5_dump_reset_stats(delay, period)` | | A snapshot labelled `a0`, then a reset. |
| `0x43` | `m5_checkpoint(delay, period)` | | Stops the host's run, labelled `a0`, for it to save a checkpoint. |
| `0x4f` | `m5_write_file(buf, len, offset, name)` | `rvsim_write_file(buf, len, offset, name)` | Writes `a1` bytes of the program's memory at `a0` to offset `a2` of the file named by the string at `a3` in the host's output directory; `a0` gets how many. |
| `0x50` | `m5_read_file(buf, len, offset)` | `rvsim_read_file(buf, len, offset)` | Copies up to `a1` bytes of the host's script file, from offset `a2`, into the program's memory at `a0`; `a0` gets how many, 0 at the end. |
| `0x51` | `m5_debug_break()` | | Stops the host's run here, labelled 0. |
| `0x52` | `m5_switch_cpu()` | | Stops the host's run here, labelled 0, for it to switch configuration. |
| `0x54` | `m5_panic()` | | Ends the run with exit code 1, with a warning. |
| `0x5a` | `m5_work_begin(id, thread)` | `rvsim_break(label)` | Stops the host's run at the marker, labelled `a0`. |
| `0x5b` | `m5_work_end(id, thread)` | | Stops the host's run at the marker, labelled `a0`. |

gem5 reads `a0` as a delay in nanoseconds for the stats operations and
`a1` as a period; rvsim applies them at once and uses `a0` as the
snapshot's label, which is 0 for a gem5 workload calling
`m5_dump_stats(0, 0)`. Any other operation number (the quiesce family,
`m5_init_param`, the symbol operations) does nothing, and a warning names
it on the `rvsim::pseudo_op` tracing target.

## Files

`readfile` and `writefile` are gem5's way of handing a guest a script to
run and getting its results back without a disk image or a network, and
rvsim serves them the same way. The host names a script file and an
output directory when it builds the simulator, and the guest reaches
them with the two operations: `rvsim readfile` inside the Linux image
copies the script to its stdout, `rvsim writefile FILE [NAME]` copies a
guest file into the output directory (under its base name by default),
and a program does the same with
the `rvsim.h` wrappers or gem5's `m5_read_file` and `m5_write_file`.

```python
from rvsim import Simulator, presets

cpu = Simulator(
    presets.linux(harts=1),
    kernel="software/linux/output/Image",
    disk="software/linux/output/disk.img",
    script="run_benchmarks.sh",   # what the guest's readfile reads
    outdir="m5out",               # where the guest's writefile writes
)
```

`Environment(script=..., outdir=...)` and the command line's `--script
FILE` and `--outdir DIR` are the same two settings. The output directory
is created if it is missing; a file name that would leave it (an
absolute path, or one with `..`) is refused.

The operations read and write the program's memory through the hart's
own translation, at its current privilege: the buffer and the name must
be mapped, with the rights a load or store would need, as they are when
the program has just touched them. They retire like a syscall in SE
mode, after every older store has reached memory, and the instructions
fetched behind them are refetched, since they may have read the memory
`readfile` filled. Without a script, `readfile` returns 0 as gem5's does
without a `readfile` parameter; without an output directory, `writefile`
returns `-ENOENT`; a buffer that is not mapped gives `-EFAULT`, and a
host file that cannot be written gives the host's `errno`, negated, as
Linux returns them.

## What the host sees

- A snapshot comes back through `Simulator.stats_dumps()`, as pairs of
  label and stats; `Simulator.stats_between(a, b)` subtracts two of them
  to give the region between. The [Session API](api.md#sessions)
  measures regions the same way.
- A stop is a `StopReason.GuestBreak` with the label from
  `Simulator.run_to(..., guest_breaks=True)`; a plain `run()` does not
  stop for it. A Session's `Marker(label)` stop point waits for one.
- An exit ends `run()` with the code.
- A file the guest wrote is in the output directory under the name it
  gave; what it read came from the script file.
