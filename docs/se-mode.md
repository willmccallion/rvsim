# SE mode: Linux user programs

SE mode runs a static Linux user program without a kernel, the way
gem5's SE mode and QEMU's user mode do: the simulator loads the ELF,
builds its page tables and stack, runs it in user mode, and serves its
syscalls itself against the host's files, clock and memory. Anything
built with a Linux cross toolchain and linked `-static` or `-static-pie`
runs unchanged:

```sh
riscv64-linux-gnu-gcc -O2 -static -march=rv64gc -mabi=lp64d prog.c -o prog
python -m rvsim ./prog -- --iterations 100 input.txt
```

Everything after the program is its command line; `--` is needed only
when the first argument starts with a dash. `--env NAME=VALUE` adds to
its environment, which is empty by default, and `--mode se` or `--mode
bare` overrides the detection: an executable whose entry lies below RAM
with no `tohost` symbol is a user program, anything else a bare-metal
image. `--script FILE` and `--outdir DIR` give the program's
[`readfile` and `writefile` pseudo-operations](pseudo-ops.md#files) their
host file and directory. From Python:

```python
from rvsim import Environment, presets

result = Environment(
    binary="prog",
    args=["--iterations", "100", "input.txt"],
    env=["LANG=C"],
    config=presets.p550(),
).run()
print(result.exit_code, result.stats["cycles"])
```

The program's exit code is the run's. One that faults, as it would with
a segmentation fault under Linux, ends the run with the signal's exit
status (139 for a page fault, 132 for an illegal instruction) and leaves
the cause, address and PC in `mcause`, `mtval` and `mepc` for a
post-mortem.

## Several programs at once

gem5's `se.py -n 4 --cmd "a;b;c;d"` runs one program per CPU with the
memory system between them, the usual way to study caches under a
multi-programmed load. rvsim does the same with `programs`: one static
program per hart from hart 0 up, each in its own address space drawn from
the same RAM, sharing whatever the configuration shares (the L3 and
memory on `presets.linux`, every cache below the cores otherwise). A hart
with no program sits parked until a program's [thread](#threads) takes
it. The run ends when the last program exits, with the first non-zero
exit code in program order, and `exit_codes` holds each program's, in the
order they were given; a program that faults ends with its signal's
status while the others go on. Each hart's stats are under its hart and
core (`hart1.*`, `core1.*`).

```python
from rvsim import Config, Program, Simulator

cpu = Simulator(
    Config(hart_count=2),
    programs=[Program("stream.elf", args=["-n", "1000000"]), Program("qsort.elf")],
)
print(cpu.run(), cpu.exit_codes)
```

```sh
python -m rvsim stream.elf --program qsort.elf -- -n 1000000   # the same, two harts
```

Every program sees the console as its stdin, stdout and stderr, so their
output interleaves as it would on one terminal; `getpid` tells them
apart (1, 2, ...). A program's `exit` ends that program; a
[pseudo-operation](pseudo-ops.md) exit (`m5_exit`, `m5_fail`) ends the
whole run at once, as in gem5.

## Threads

A program's threads run the way gem5's SE mode runs them: `clone` starts
a thread on the lowest-numbered hart no thread holds, and the thread
keeps that hart until it exits. A hart runs one thread at a time and
nothing is time-sliced, so a program needs a hart for each thread it has
at once; with none free, `clone` fails with `EAGAIN`, which
`pthread_create` returns. Give the run a hart per thread, as gem5's
`se.py -n N` gives it CPUs:

```sh
python -m rvsim ./workers --harts 5 -- 4   # the first thread and four workers
```

```python
from rvsim import Config, Simulator

cpu = Simulator(Config(hart_count=5), binary="workers", args=["workers", "4"])
```

A hart whose thread has exited takes the next new one, of any program.
A process's threads share its address space, heap, mappings and
descriptors; each has its own registers, its stack, and the thread
pointer `clone` gave it, and the memory system keeps the harts' caches
coherent as it does for any harts. `gettid` gives each thread its own
id and `getpid` its process's, which is the first thread's.
`sched_getaffinity` reports every hart, and so do the CPU lists a
program sizes its thread pool by: `/sys/devices/system/cpu/online`,
`possible` and `present` name the simulated harts, not the host's CPUs,
as gem5 serves them, so `sysconf(_SC_NPROCESSORS_ONLN)` and
`get_nprocs()` return the hart count.

A thread waiting in `futex`, as `pthread_mutex_lock`, `pthread_cond_wait`,
`pthread_join` and barriers do, parks its hart until another thread
wakes it or its timeout passes in simulated time; the parked hart costs
the run nothing. `sched_yield` returns at once, there being nothing else
to run on the hart. When every thread waits on a futex with no timeout,
nothing can wake them: rvsim warns, and the run goes on to its cycle
limit, as the program would hang under Linux.

`exit`, which `pthread_exit` calls, ends one thread: its
`set_tid_address` word is zeroed and a waiter on it woken, which is how
`pthread_join` returns. `exit_group`, which `exit()` and a return from
`main` call, ends every thread of the process at once, and so does a
fault or a fatal signal in any of them. When the last thread of a process
ends with `exit`, the process's exit code is its first thread's, as Linux
reports it.

When a thread unmaps memory or changes a mapping's permissions, the
process's other threads drop their translations at their harts' next
commit, as a kernel's TLB shootdown makes them; the interrupt a real
shootdown costs is not charged. A checkpoint carries every thread, a
waiting one with its timeout.

## What is modelled

The program runs under real paging: the loader builds Sv39 tables by
default (`Config(page_mode="Sv48")` or `"Sv57"` selects the others), so
TLB misses and page walks have their full timing, which gem5's SE mode
never gave. Every page is mapped eagerly at load, `brk` and `mmap`, so
there are no page faults to serve and a run is deterministic.

A syscall costs the pipeline what a serialising instruction costs: the
`ecall` retires when every older instruction has and every older store
has reached memory, the simulator performs the call, and the
instructions fetched behind it are refetched, since they may have read
`a0` or a mapping the call changed. There is no kernel to charge for the
call itself, so OS-heavy workloads belong in full-system mode; this is
the trade every SE mode makes.

Time comes from the simulated clock: `clock_gettime`, `gettimeofday`,
`times` and `getrusage` divide the cycle count by `Config(clock_hz=...)`,
1 GHz by default, so a program that times itself measures the simulated
machine. `CLOCK_REALTIME` starts from a fixed epoch, and `getrandom`
draws from a fixed-seed generator, so two runs of the same program see
the same values.

The program's stdin, stdout and stderr are the simulator's console, as
the UART is for a bare-metal program or a kernel, so `Config(console=...)`
applies to them: `"quiet"` drops the output, `"captured"` keeps it for
`read_console()` and makes `read` return what `write_console()` typed,
and otherwise the host's stdin feeds `read`, which waits for it as it
would under Linux. A bare-metal program printing through HTIF uses the
same console.

A program marks the region it wants measured, or stops and ends the run,
with [pseudo-operations](pseudo-ops.md), the same m5op instruction a
gem5 workload uses.

## The syscalls served

The Linux riscv64 ABI (`asm-generic/unistd.h`), as static glibc and
newlib programs use it: `read`, `write`, `readv`, `writev`, `pread64`,
`pwrite64`, `openat`, `close`, `lseek`, `fstat`, `newfstatat`,
`faccessat`, `readlinkat`, `mkdirat`, `unlinkat`, `renameat`,
`renameat2`, `getcwd`, `fcntl` and `ioctl` (the console answers
`TCGETS` and `TIOCGWINSZ`); `brk`, `mmap` (anonymous, and private file
mappings as a copy), `munmap`, `mremap`, `mprotect`, and `madvise` (the
advice a 6.6 kernel knows is accepted and does nothing; other advice is
refused, as glibc 2.42's guard-page request is by that kernel); `clone`
for threads (`CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND |
CLONE_THREAD`, with `CLONE_SETTLS`, `CLONE_PARENT_SETTID`,
`CLONE_CHILD_SETTID` and `CLONE_CHILD_CLEARTID`), `exit` and
`exit_group`; `futex` (`FUTEX_WAIT`, `FUTEX_WAKE` and their bitset forms,
private or shared, with timeouts on either clock); `kill`, `tkill` and
`tgkill` (a signal ends the process with its exit status unless it is 0,
which checks the target exists, or one whose default is to be ignored;
handlers never run); `clock_gettime`, `clock_getres`, `gettimeofday`,
`times`, `getrusage`, `nanosleep` and `clock_nanosleep` (no time passes);
`uname`, `sysinfo`, `prlimit64`, `getrandom`; `getpid`, `gettid`,
`getppid` and the uid and gid calls; `set_tid_address`,
`sched_getaffinity` and `sched_yield`; `set_robust_list`,
`rt_sigaction`, `rt_sigprocmask` and `riscv_flush_icache`, which are
accepted and do nothing; `riscv_hwprobe`, answered as the 6.6 kernel
`uname` reports would answer for this hart (the base behaviour, the
extension bits that kernel knows from the hart's ISA string, and whether
misaligned accesses are fast), so glibc selects the same routines it
would under Linux on rvsim. `clone3`, `execve`, `statx` and `rseq` return
`ENOSYS`, which glibc falls back from (`pthread_create` to `clone`).
Anything else returns `ENOSYS` with a warning. Files are the host's,
relative to the working directory, as in gem5, except the CPU lists under
`/sys/devices/system/cpu`, which describe the simulated machine.

`RUST_LOG=rvsim::syscall=trace` logs every call with its arguments and
result, like `strace`.

## How it is checked

`make se-verify` runs the check programs in `tools/se/programs` (argv
and the environment, the heap through `brk`, `mmap` and `mremap`, host
files, the clocks, and pthreads on five harts: a contended mutex, a
condition variable, a barrier, thread-local storage, joins, a second
round of threads on the harts the first freed, and a timed wait) under
rvsim and under `qemu-riscv64`, and requires
the same output and exit code from both; `EMBENCH=1` adds the 19
Embench-IoT benchmarks (`make embench-build`), each of which verifies
its own result and exits 0, and `BUNDLED=1` the bundled programs and
benchmarks (`make software`), compared on everything but the cycle
counts they print; `life` and `sand` take minutes under rvsim and join
only with `SLOW=1`, and `regions` is left out because qemu traps on its
[pseudo-operations](pseudo-ops.md). CI runs the check programs, Embench
and the bundled set on every change.

## Limits

One thread per hart and no time-slicing, so a program needs as many harts
as it has threads at once (`clone` fails with `EAGAIN` beyond them).
`clone` makes threads only, not processes: `fork`, `vfork` and the
`clone` behind `posix_spawn` and `system` return `ENOSYS`. Signal handlers
never run, and the futex requeue, wake-op and priority-inheritance
operations return `ENOSYS` (glibc's default mutexes and condition
variables do not use them). Dynamically linked executables are refused
with a message saying to link `-static` or `-static-pie`. A checkpoint
keeps each process's mappings, break, heap and threads but not its host
files, so one is refused while a program has a file open.
