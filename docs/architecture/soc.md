# SoC Devices

rvsim models a complete system-on-chip based on the QEMU `virt` machine layout. This is the same memory map that Linux and OpenSBI expect, allowing unmodified firmware and kernels to boot.

## Memory Map

| Device | Base Address | Size | Description |
|--------|-------------|------|-------------|
| SYSCON | `0x0010_0000` | 8B | System controller (poweroff/reboot) |
| Goldfish RTC | `0x0010_1000` | 4KB | Real-time clock |
| CLINT | `0x0200_0000` | 64KB | Core Local Interruptor |
| PLIC | `0x0C00_0000` | 64MB | Platform-Level Interrupt Controller |
| UART | `0x1000_0000` | 4KB | 16550A serial port |
| RAM | `0x8000_0000` | configurable | Main memory (default: 256MB) |
| VirtIO Disk | `0x9000_0000` | 4KB | Block device |

## Devices

### CLINT (Core Local Interruptor)

Timer subsystem providing `mtime` and `mtimecmp` registers:

- `mtime` increments every `clint_divider` CPU cycles (default: 10)
- When `mtime >= mtimecmp`, a timer interrupt is raised (MIP.MTIP)
- Timer interrupts can be delegated to S-mode via `mideleg`

### PLIC (Platform-Level Interrupt Controller)

Priority-based interrupt controller with:

- 53 interrupt sources
- 2 contexts: M-mode and S-mode
- Per-source priority registers
- Per-context enable bits and priority threshold
- Claim/complete protocol: reading the claim register returns the highest-priority pending interrupt and clears it

### UART (16550A)

Serial port compatible with the NS16550A register interface:

- Transmit and receive holding registers
- Interrupt enable register (IER) with receive data available and transmit holding register empty interrupts
- Line status register (LSR) with data ready and transmitter empty bits
- FIFO control register (FCR)

UART output can be directed to stdout (default), stderr (`uart_to_stderr=True`), or suppressed entirely (`uart_quiet=True`).

### VirtIO MMIO Block Device

VirtIO specification-compliant block device:

- MMIO transport (VirtIO version 2)
- Single virtqueue for block I/O requests
- Read and write operations via DMA from/to guest memory
- Backed by a host file (e.g., a rootfs image)
- Interrupt notification via PLIC

Used to mount the root filesystem when booting Linux.

### Goldfish RTC

Real-time clock providing wall-clock time:

- Read-only `TIME_LOW` and `TIME_HIGH` registers
- Returns host system time in nanoseconds
- Used by Linux for initial system time setup

### SYSCON (System Controller)

Simple system controller recognizing two magic values:

- Write `0x5555` → **poweroff** (simulation exits with code 0)
- Write `0x7777` → **reboot** (simulation restarts)

### HTIF (Host-Target Interface)

Berkeley Host-Target Interface for `riscv-tests` compatibility:

- `tohost` / `fromhost` memory-mapped registers at a configurable address
- Writing `tohost = 1` signals test pass; `tohost > 1` signals test failure
- Supports basic syscall proxying for bare-metal programs

The HTIF address is automatically detected from the ELF binary's symbol table.

## Device Tree

rvsim auto-generates a Flattened Device Tree Blob (DTB) from the active configuration. The DTB is placed at `ram_base + 0x0220_0000` and includes:

- CPU node with ISA string matching the enabled extensions
- Memory node sized to `ram_size`
- Device nodes for all SoC devices at their configured addresses
- `chosen` node with bootargs and initrd location (for Linux boot)

The DTB is passed to firmware (OpenSBI) via the `a1` register at boot, following the standard RISC-V boot protocol.

## Boot Flow

1. CPU starts at `ram_base` (`0x8000_0000`) in M-mode
2. OpenSBI firmware initializes, sets up trap delegation to S-mode
3. OpenSBI jumps to the kernel at `ram_base + kernel_offset` (`0x8020_0000`)
4. Linux kernel initializes, mounts VirtIO rootfs, launches init (BusyBox)
5. Login prompt appears on UART
