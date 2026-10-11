# Pseudo-operations

A program running on rvsim can ask the simulator for things no hardware
does: reset the statistics, take a labelled snapshot of them, stop the
host's run at a marker, or end the simulation with an exit code. It asks
with a pseudo-instruction, the one gem5's m5ops use, so the same source
serves a bare-metal image, a Linux user program under
[SE mode](se-mode.md) and a program inside a Linux guest, and a binary
instrumented with gem5's `libm5` runs on rvsim unchanged.

## The instruction

The m5op instruction uses the `custom-3` major opcode, `0x7b`, with the
operation number in bits 31:25 and the other fields zero:

```
31        25 24                    7 6       0
+-----------+-----------------------+---------+
| operation |           0           | 1111011 |
+-----------+-----------------------+---------+
```

Its arguments are in `a0` and `a1`, as gem5's are. The instruction is
legal at every privilege level, costs one issue slot, retires
non-speculatively like a system instruction, and its effect lands at the
end of the cycle it retires, together with the sim-control device's
commands. Spike and real hardware trap on it as an illegal instruction,
so a program meant to run there too should keep its pseudo-operations
behind a build flag.

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
| `0x21` | `m5_exit(delay)` | | Ends the run with exit code 0. |
| `0x22` | `m5_fail(delay, code)` | `rvsim_exit(code)` | Ends the run with exit code `a1`. |
| `0x40` | `m5_reset_stats(delay, period)` | `rvsim_reset_stats()` | Zeroes the stats; the window measured from here. |
| `0x41` | `m5_dump_stats(delay, period)` | `rvsim_dump_stats(label)` | Keeps a snapshot of the stats labelled `a0`. |
| `0x42` | `m5_dump_reset_stats(delay, period)` | | A snapshot labelled `a0`, then a reset. |
| `0x43` | `m5_checkpoint(delay, period)` | | Stops the host's run, labelled `a0`, for it to save a checkpoint. |
| `0x5a` | `m5_work_begin(id, thread)` | `rvsim_break(label)` | Stops the host's run at the marker, labelled `a0`. |
| `0x5b` | `m5_work_end(id, thread)` | | Stops the host's run at the marker, labelled `a0`. |

gem5 reads `a0` as a delay in nanoseconds for the stats operations and
`a1` as a period; rvsim applies them at once and uses `a0` as the
snapshot's label, which is 0 for a gem5 workload calling
`m5_dump_stats(0, 0)`. Any other operation number does nothing, and a
warning names it on the `rvsim::pseudo_op` tracing target.

## What the host sees

- A snapshot comes back through `Simulator.stats_dumps()`, as pairs of
  label and stats; `Simulator.stats_between(a, b)` subtracts two of them
  to give the region between. The [Session API](api.md#sessions)
  measures regions the same way.
- A stop is a `StopReason.GuestBreak` with the label from
  `Simulator.run_to(..., guest_breaks=True)`; a plain `run()` does not
  stop for it. A Session's `Marker(label)` stop point waits for one.
- An exit ends `run()` with the code.

## The sim-control device

The same operations exist as an MMIO device at `sim_control_base`
(default `0x0010_2000`), outside the device tree, with two 64-bit
registers:

| Offset | Register | Access |
|---|---|---|
| `0x00` | `COMMAND` | write `1` to reset the stats, `2` to dump them labelled with `ARG`, `3` to end the simulation with `ARG` as the exit code, `4` to stop the host's `run_to` here with `ARG` as the label |
| `0x08` | `ARG` | read/write: the argument of the next command |

A guest writes `ARG`, then `COMMAND`, and the command takes effect at the
end of the cycle the write reaches the device. The Linux image's `rvsim`
tool (`rvsim dump-stats LABEL`, `rvsim break LABEL`,
`rvsim run START END CMD`) drives it through `/dev/mem`.
