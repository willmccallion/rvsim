# SoC Devices

rvsim models a complete system-on-chip based on the QEMU `virt` machine layout. This is the same memory map that Linux and OpenSBI expect, allowing unmodified firmware and kernels to boot.

## Memory Map

| Device | Base Address | Size | Description |
|--------|-------------|------|-------------|
| SYSCON | `0x0010_0000` | 4KB | System controller (power-off, reset, failure) |
| Goldfish RTC | `0x0010_1000` | 4KB | Real-time clock |
| Sim control | `0x0010_2000` | 4KB | Guest control of statistics and the run |
| CLINT | `0x0200_0000` | 64KB | Core Local Interruptor |
| PLIC | `0x0C00_0000` | 64MB | Platform-Level Interrupt Controller |
| UART | `0x1000_0000` | 4KB | 16550A serial port |
| RAM | `0x8000_0000` | configurable | Main memory (default: 256MB) |
| VirtIO Disk | `0x9000_0000` | 4KB | Block device |
| HTIF | the ELF's `tohost` symbol | 16B | Test pass/fail for riscv-tests |

The bases of RAM, the UART, the disk, the CLINT, SYSCON and sim control
are configurable (see [Configuration](../configuration.md#system)); the
RTC and PLIC are fixed. `presets.linux()` moves the disk to `0x1000_1000`,
beside the UART, where the bundled image's device tree expects it.

## Device access timing

Devices sit on the system bus, so an uncached access pays `bus_latency`
each way, and every device takes `device_latency_ns` (100 ns by default)
to answer a register access, converted to cycles at `cpu_clock_mhz`.
`device_latency_ns_overrides` sets the time per device, by the names
`UART0`, `CLINT`, `PLIC`, `VirtIO-Blk`, `SysCon`, `GoldfishRTC` and
`HTIF`. Device accesses bypass the caches, and a load from a device waits
until it is the oldest instruction, so it is never performed on a wrong
path.

## Devices

### CLINT (Core Local Interruptor)

Timer and software-interrupt registers for every hart, at the standard
layout (`msip` at `0x0 + 4·hart`, `mtimecmp` at `0x4000 + 8·hart`, one
`mtime` at `0xBFF8`):

- `mtime` increments every `clint_divider` CPU cycles (default: 10)
- The device tree advertises a 10 MHz timebase. `presets.linux()` sets
  `clint_divider` to `cpu_clock_mhz / 10` so `mtime` runs at that rate and
  the guest's clock keeps simulated time; with another divider the guest's
  time runs fast or slow by the ratio
- When `mtime >= mtimecmp[hart]`, that hart's timer interrupt is raised (MIP.MTIP)
- Writing `msip[hart]` raises that hart's software interrupt (MIP.MSIP); this
  is how firmware sends inter-processor interrupts
- Timer interrupts can be delegated to S-mode via `mideleg`

### PLIC (Platform-Level Interrupt Controller)

Priority-based interrupt controller with:

- 53 interrupt sources advertised in the device tree (the UART is source
  10 and the disk source 1)
- 2 contexts per hart: context `2·hart` is the hart's M-mode target and
  `2·hart + 1` its S-mode target (enables at `0x2000 + 0x80·context`,
  threshold and claim at `0x200000 + 0x1000·context`)
- Per-source priority registers
- Per-context enable bits and priority threshold
- Claim/complete protocol: reading the claim register returns the highest-priority pending interrupt and clears it

Every cycle the bus ticks each device once, feeds the active sources into
the PLIC, and samples one set of lines per hart (`mtip`, `msip`, `meip`,
`seip`) that the hart folds into its `mip` before its pipeline ticks. A
change in the PLIC's pending, enable or threshold state reaches the lines
and the claim registers three cycles later, as in gem5's PLIC.

### UART (16550A)

Serial port compatible with the NS16550A register interface:

- Transmit and receive holding registers
- Interrupt enable register (IER) with receive data available and transmit holding register empty interrupts
- Line status register (LSR) with data ready and transmitter empty bits
- FIFO control register (FCR)
- Output leaves as it is written; the transmit-empty and receive-data
  interrupts rise 225 ns after their cause, as in gem5's `Uart8250`

`console` connects the UART to stdout (the default), stderr, nothing
(`"quiet"`), or an in-memory buffer (`"captured"`) that the host reads with
`read_console()` and writes input to with `write_console()`; Linux sessions
use the captured console to type commands at the shell.

### VirtIO MMIO Block Device

VirtIO specification-compliant block device:

- MMIO transport (VirtIO version 2)
- Single virtqueue for block I/O requests
- Read and write operations via DMA from/to guest memory: a request's
  DMA moves over the system bus and memory controller phase by phase (the
  ring and descriptor reads, the data in line-sized transfers, then the
  status and used-ring writes), and the request completes when its last
  transfer returns
- Backed by a host file (e.g., a rootfs image)
- Interrupt notification via PLIC, raised when the request completes

Used to mount the root filesystem when booting Linux.

### Goldfish RTC

Real-time clock providing wall-clock time:

- Read-only `TIME_LOW` and `TIME_HIGH` registers, in nanoseconds
- Reports `rtc_epoch_seconds` (2026-01-01 by default) plus the simulated
  time elapsed at `cpu_clock_mhz`, never the host's clock, so every run
  reads the same time
- Used by Linux to set the system time at boot

### SYSCON (System Controller)

A 32-bit command register at offset 0, as QEMU's `virt` machine has:

| Value | Meaning | Exit code |
|-------|---------|-----------|
| `0x5555` | Power off | 0 |
| `0x7777` | Reset, taken as an exit | 0 |
| `0x3333` | Failure | 1 |

Linux's `poweroff` and `reboot` reach it through OpenSBI's system-reset
call.

### Sim control

Lets software in the guest mark what the host should measure, as gem5's
`m5` operations do. Write the argument to `ARG` (offset `0x08`), then the
command to `COMMAND` (offset `0x00`):

| Command | Effect |
|---------|--------|
| `1` | Reset the statistics |
| `2` | Keep a snapshot of the statistics labelled `ARG`; the host subtracts two snapshots to get the region between them |
| `3` | End the simulation with exit code `ARG` |
| `4` | Stop the host's run here, labelled `ARG`, so it can save, switch configuration or measure from this point |

Bare-metal programs use the inline functions in `software/libc/rvsim.h`.
In a Linux guest, the `rvsim` tool (`software/guest/rvsim.c`, installed in
the bundled root filesystem) maps the device through `/dev/mem`;
`rvsim run START END CMD` snapshots the statistics around one command,
which is how `Session.measure()` and `rvsim bench` delimit a benchmark.

### HTIF (Host-Target Interface)

Berkeley Host-Target Interface for `riscv-tests` compatibility:

- A 16-byte slot at the address of the ELF's `tohost` symbol, found when
  the program is loaded
- Writing `tohost = 1` ends the run with exit code 0; an odd value above 1
  is a failure, with the failing test number in `value >> 1`
- Writing 0 is ignored, as the tests clear `tohost` before reporting

There is no system-call proxying. Bare-metal programs without a trap
handler end with `ecall`: with `a7 = 93` (exit) the run ends with `a0` as
its exit code.

## Device Tree

rvsim generates a Flattened Device Tree Blob (DTB) from the active
configuration, unless a DTB file is supplied. It is placed at
`ram_base + 0x0220_0000` and contains:

- One `cpu@N` node per hart, with the `misa` letters plus `_sstc` (and
  `_svadu` when enabled) as its `riscv,isa`, `riscv,sv39` as its MMU type,
  and a `cpu-map`
- A memory node sized to `ram_size`
- The CLINT, with timer and software interrupts for every hart, and the
  PLIC, with an M- and an S-mode context for every hart
- The UART, the VirtIO disk, SYSCON with `syscon-poweroff` and
  `syscon-reboot` nodes, and the RTC
- `timebase-frequency` and a `chosen` node with the boot arguments
  (`root=/dev/vda rw console=ttyS0`, with an early console on the UART)

The DTB's address is passed to the firmware in `a1`, following the
standard RISC-V boot protocol.

## Boot Flow

1. OpenSBI is loaded at `ram_base`, the kernel image at
   `ram_base + 0x20_0000` and the DTB at `ram_base + 0x0220_0000`.
2. Every hart starts in OpenSBI in M-mode with its hart id in `a0` and the
   DTB in `a1`. For the `fw_dynamic` firmware, `a2` points to the
   `fw_dynamic_info` record naming the kernel's address and S-mode.
3. OpenSBI's boot lottery picks one hart to initialise the platform and
   parks the rest, delegates traps to S-mode, and enters the kernel.
4. Linux brings the other harts up through the SBI hart-state calls, mounts
   the VirtIO root filesystem and starts BusyBox's init.
5. A shell prompt appears on the UART.

Without firmware the loader instead places an `MRET` at `ram_base` whose
`mepc` is a kernel loaded at `ram_base + kernel_offset`; `kernel_offset`
has no effect when OpenSBI is used.
