# Linux Boot

rvsim boots Linux 6.6 through OpenSBI to a BusyBox shell, on one core or
on several kept coherent by the MESI fabric, with either backend. A boot is
also the starting point for measuring real software:
[Linux Benchmarks](linux-benchmarks.md) runs benchmarks from a cached
boot.

## Building the image

```bash
make linux
```

This downloads Buildroot and builds, into `software/linux/output/`:

| File | What it is |
|------|------------|
| `fw_jump.bin`, `fw_dynamic.bin` | OpenSBI firmware, loaded at `0x8000_0000` |
| `Image` | The Linux 6.6 kernel, loaded at `0x8020_0000` |
| `disk.img` | The root filesystem: BusyBox, the benchmark suite and the guest `rvsim` tool, mounted from the VirtIO disk |

The image is described by three files in `software/guest/`:
`buildroot.defconfig` (Buildroot 2024.08, kernel 6.6.44, OpenSBI and the
packages), `linux.fragment` (the kernel options the simulated machine
relies on, applied over the kernel's default configuration: the vector
extension, Zicbom and Zicboz, perf events and the SBI PMU driver) and
`post_build.sh` (compiles the guest tool and STREAM into the root
filesystem and sets `kernel.perf_user_access = 2` so `rdcycle` works
from user space). The first build takes 15 to 30 minutes; later builds
reuse Buildroot's cache. Outside the Nix development shell, `make linux`
runs the build in the repository's FHS environment. `rvsim
tools/boot_linux.py --rebuild` rebuilds the image after a change to
these files.

### The guest tool

The image's `rvsim` command talks to the simulator with the
[pseudo-instruction](../pseudo-ops.md), as gem5's `m5` tool does:

| Command | What it does |
|---|---|
| `rvsim dump-stats LABEL` | Keeps a snapshot of the stats labelled `LABEL` |
| `rvsim reset-stats` | Zeroes the stats |
| `rvsim break LABEL` | Stops the host's `run_to` here, labelled `LABEL` |
| `rvsim exit [CODE]` | Ends the simulation |
| `rvsim run START END CMD [ARG...]` | Dumps `START`, runs `CMD`, dumps `END`: what `Session.measure` uses |
| `rvsim readfile` | Copies the host's script file (`Simulator(script=...)`) to stdout |
| `rvsim writefile FILE [NAME]` | Copies `FILE` to `NAME` (its base name by default) in the host's output directory (`Simulator(outdir=...)`) |

A boot with a script and an output directory runs the script and
collects its results without a shell session:

```python
cpu = Simulator(
    presets.linux(harts=1),
    kernel="software/linux/output/Image",
    disk="software/linux/output/disk.img",
    script="bench.sh",
    outdir="m5out",
)
```

```sh
# bench.sh, run in the guest as: rvsim readfile > /tmp/bench.sh; sh /tmp/bench.sh
rvsim run 1 2 coremark 0x0 0x0 0x66 200 7 1 2000 > coremark.log
rvsim writefile coremark.log
rvsim exit
```

## Booting from the command line

```bash
make run-linux                         # eight cores
make run-linux HARTS=1                 # one core
rvsim tools/boot_linux.py --harts 2 --interconnect ring
rvsim tools/boot_linux.py --memory dram          # row-buffer DRAM instead of DDR5
rvsim tools/boot_linux.py --speed-bin 4800B      # a slower DDR5 bin
```

| Option | Meaning |
|--------|---------|
| `--harts N` | Cores to boot (default 8) |
| `--interconnect NAME` | `crossbar`, `ring`, `mesh` (default), `torus` or `hypercube` |
| `--memory ddr5\|dram` | JEDEC DDR5 with four channels (default), or the row-buffer DRAM model |
| `--speed-bin BIN` | DDR5 speed bin, `4800B` or `5600B` (default) |
| `--no-build` | Fail instead of building when the image is missing |
| `--no-boot` | Build only |
| `--rebuild` | Rebuild the image even if one exists |

The machine is `presets.linux()`: `Config()` cores (the default machine's
6-wide out-of-order core with Seznec's 64KB TAGE-SC-L), private L1 and L2
caches kept coherent by a snoop-filter home agent, a shared L3, and DDR5.
The console is the terminal: log in as `root` with no password.

```
Welcome to Buildroot
buildroot login: root
#
```

Each core is simulated cycle by cycle, so an eight-core boot takes several
times as long as a one-core boot.

## Booting from Python

`Simulator` boots a kernel when given one. Without `firmware=`, it uses
`software/linux/output/fw_jump.bin` (or `fw_dynamic.bin`) relative to the
working directory, so run from the repository root:

```python
from rvsim import Simulator, presets

cpu = Simulator(
    presets.linux(harts=1),
    kernel="software/linux/output/Image",
    disk="software/linux/output/disk.img",
)
cpu.run()
```

`firmware=` names another OpenSBI build and `dtb=` a device tree to use in
place of the generated one. Any core works inside the Linux system,
including the in-order backend:

```python
from rvsim import presets

in_order = presets.linux(harts=1, core=presets.inorder())
boom = presets.linux(harts=1, core=presets.boom())
```

### Sessions: boot once, measure many times

`Session` drives a workload in phases and caches its fast-forwards, which
is the practical way to study software running on Linux:

```python
from rvsim import Session, presets

s = Session.linux(harts=1)
s.fast_forward(until=Session.LOGIN_SHELL)        # boots once, then restored from the cache
s.switch(presets.linux(harts=1, core=presets.inorder()))
s.warm_up(command="coremark 0x0 0x0 0x66 20 7 1 2000")
r = s.measure("coremark 0x0 0x0 0x66 20 7 1 2000")
print(r.cycles, r.instructions, r.ipc, r.exit_code)
print(r.stats["core0.bp.committed.accuracy"])

out = s.shell("uname -a")                        # run a command and read its output
print(out.output)
```

- **`fast_forward(until=...)`** reaches a stop quickly: on the first run it
  boots on a fast-forward configuration whose timer ticks every cycle,
  which shortens the boot's sleeps, and saves a checkpoint; later runs
  restore it. The cache key covers the simulator build, the image files,
  everything the session did before, the configuration and the stop, so a
  boot made by one build is never restored under another.
- **`switch(config)`** continues on another core configuration of the same
  system (same memory map, harts and RAM); its caches, TLBs and predictors
  start cold.
- **`warm_up()`** runs unmeasured, to warm them.
- **`measure(command)`** runs a shell command under the guest's
  `rvsim run`, which snapshots the statistics just before the command
  starts and just after it exits, and returns the difference as a
  `Region`. `measure(until=stop)` measures a run to a stop instead.
- **`send()`, `expect()` and `shell()`** type into the console and wait for
  its output.

## Boot sequence

1. OpenSBI starts on every hart in M-mode, with its hart id in `a0` and
   the generated device tree in `a1`. Its boot lottery picks one hart to
   initialise the platform and parks the others.
2. OpenSBI delegates traps and interrupts to S-mode, enables Sstc, and
   jumps to the kernel at `0x8020_0000` in S-mode.
3. Linux sets up Sv39 page tables, starts the timer, PLIC, UART and VirtIO
   drivers, and brings the other harts online through OpenSBI's hart-state
   calls.
4. It mounts the VirtIO root filesystem and starts BusyBox's init, which
   runs a login shell on the UART.

The memory map, devices and device tree are described in
[SoC Devices](../architecture/soc.md).

## Boot measurements

One hart booting from reset to the login shell on `presets.linux(harts=1)`,
with the `fast()` core and the same core switched to the in-order
backend, measured 2026-10-11 with `Session.measure(until=Session.LOGIN_SHELL)`
and the timer at its real 10 MHz rate:

| | Out-of-order | In-order |
|---|---:|---:|
| Cycles to the shell | 693.1 M | 785.1 M |
| Cycles in `WFI` | 484.9 M (70%) | 475.8 M (61%) |
| Instructions retired | 379.2 M | 314.8 M |
| IPC over the boot | 0.55 | 0.40 |
| IPC outside `WFI` | 1.82 | 1.02 |
| Cycles in S / U / M mode | 98% / 2% / 0.4% | 96% / 3% / 0.7% |
| Branch accuracy at commit | 98.9% | 98.8% |
| L1I / L1D / L2 miss rate | 0.7% / 1.5% / 23.4% | 0.7% / 1.2% / 21.9% |
| Host time | 879 s | 794 s |
| Simulated cycles per host second | 0.79 M | 0.99 M |

Most of a boot is spent waiting: the kernel sleeps in `WFI` for timers
and for the disk, and the simulator skips those idle cycles instead of
ticking them, so the host time follows the active cycles. The two
backends retire different instruction counts because the kernel's
busy-waits and calibration loops run for a time, not a count. The
out-of-order core's main stalls outside `WFI` are operands not ready
(42 M cycles), no free branch checkpoint (30 M) and rename
waiting behind serializing instructions (29 M); the in-order
core's are operands not ready and dispatch into a full issue queue
(83 M).

The runs are deterministic: the in-order boot, run twice, gave the same
cycle count and statistics both times. Every additional hart is another
core simulated cycle by cycle, so an eight-hart boot takes correspondingly
longer on the host.
