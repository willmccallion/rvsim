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
mappings as a copy), `munmap`, `mremap`, `mprotect`, `madvise`; `exit`,
`exit_group`, `kill`, `tkill` and `tgkill` (the signal's exit status);
`clock_gettime`, `clock_getres`, `gettimeofday`, `times`, `getrusage`,
`nanosleep` and `clock_nanosleep` (no time passes); `uname`, `sysinfo`,
`prlimit64`, `getrandom`; `getpid`, `gettid`, `getppid` and the uid and
gid calls; `set_tid_address`, `set_robust_list`, `futex`,
`rt_sigaction`, `rt_sigprocmask`, `sched_yield` and `riscv_flush_icache`
as a single thread needs them; `riscv_hwprobe`, answered as the 6.6 kernel
`uname` reports would answer for this hart (the base behaviour, the
extension bits that kernel knows from the hart's ISA string, and whether
misaligned accesses are fast), so glibc selects the same routines it
would under Linux on rvsim. `clone`, `clone3`, `execve`, `statx` and
`rseq` return `ENOSYS`, which glibc falls back from; so threads are not
supported yet. Anything else returns `ENOSYS` with a
warning. Files are the host's, relative to the working directory, as in
gem5.

`RUST_LOG=rvsim::syscall=trace` logs every call with its arguments and
result, like `strace`.

## How it is checked

`make se-verify` runs the check programs in `tools/se/programs` (argv
and the environment, the heap through `brk`, `mmap` and `mremap`, host
files, the clocks) under rvsim and under `qemu-riscv64`, and requires
the same output and exit code from both; `EMBENCH=1` adds the 19
Embench-IoT benchmarks (`make embench-build`), each of which verifies
its own result and exits 0, and `BUNDLED=1` the bundled programs and
benchmarks (`make software`), compared on everything but the cycle
counts they print; `life` and `sand` take minutes under rvsim and join
only with `SLOW=1`, and `regions` is left out because qemu traps on its
[pseudo-operations](pseudo-ops.md). CI runs the check programs, Embench
and the bundled set on every change.

## Limits

One hart; dynamically linked executables are refused with a message
saying to link `-static` or `-static-pie`. A checkpoint keeps the process's mappings, break and heap but
not its host files, so one is refused while the program has a file
open.
