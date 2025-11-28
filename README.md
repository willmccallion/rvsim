# RISC-V System Emulator & Microkernel

A cycle-accurate RISC-V (RV64IM) processor simulator written in Rust, capable of booting a custom C-based microkernel. The project features a 5-stage pipeline with advanced architectural features like branch prediction, a 3-level cache hierarchy, and MMU simulation.

The included operating system provides a shell, a read-only virtual file system, and the ability to load and execute user-space programs dynamically.

## Features

### CPU Architecture
*   **ISA:** RV64IM (Integer + Multiply/Divide extensions).
*   **Pipeline:** 5-Stage (Fetch, Decode, Execute, Memory, Writeback) with hazard detection and forwarding.
*   **Branch Prediction:**
    *   GShare predictor with Global History Register (GHR).
    *   Branch Target Buffer (BTB).
    *   Return Address Stack (RAS) for function calls.
*   **Memory Hierarchy:**
    *   **L1 Cache:** Split Instruction (16KB) and Data (16KB), 4-way set associative.
    *   **L2 Cache:** Unified 128KB, 8-way set associative.
    *   **L3 Cache:** Unified 2MB, 16-way set associative.
    *   **MMU:** Sv39-style translation simulation with TLB latency penalties. (Work in progress)
*   **Privilege Modes:** Machine (M-Mode), Supervisor (S-Mode), and User (U-Mode).

### Operating System (Microkernel)
*   **Bootloader:** Assembly-based bootloader (`boot.s`) that loads the kernel from the virtual disk.
*   **Kernel:** C-based microkernel (`main.c`) handling:
    *   UART I/O drivers.
    *   Virtual File System (VFS) parsing.
    *   ELF-like binary loading.
    *   Trap/Interrupt delegation and handling.
    *   `malloc`/`free` memory management.
*   **User Space:** A command-line shell (`sh`) capable of listing files (`ls`) and executing programs.

## Directory Structure

*   `src/`: Rust source code for the CPU simulator.
*   `software/bootloader/`: Assembly source for the M-Mode bootloader.
*   `software/kernel/`: C source for the S-Mode kernel.
*   `software/user/`: C source for user-space programs (games, benchmarks).
*   `bin/`: Compiled user binaries (automatically packed into the disk image).

## Requirements

1.  **Rust:** Latest stable Cargo.
2.  **RISC-V Toolchain:** `riscv64-unknown-elf-gcc` (or `riscv64-linux-gnu-gcc`).
    *   *Note: The build script (`build.rs`) automatically detects and uses the toolchain to compile the kernel and bootloader.*

## Building and Running

### 1. Build the Simulator & Kernel
The Rust build script handles the compilation of the bootloader and kernel automatically.
```bash
cargo build --release
```

### 2. Compile User Programs
You must compile the user programs (found in the prompt or your `software/user` dir) into raw binaries and place them in the `bin/` directory. The simulator reads this directory to construct the virtual disk.

*Example (assuming you have a Makefile or compile manually):*
```bash
# Compile your C programs to flat binaries and place them here:
mkdir -p bin
# (Perform compilation of life.c, sand.c, etc. -> bin/life.bin, bin/sand.bin)
```

### 3. Run the System
```bash
cargo run --release
```

To enable cycle-by-cycle pipeline tracing:
```bash
cargo run --release -- --trace
```

## Usage

Once the simulator starts, it will boot the kernel and drop you into a shell:

```text
root@riscv:~# help
Built-ins: ls, help, clear, exit
root@riscv:~# ls
-r-x       16200    life.bin
-r-x       14500    sand.bin
-r-x        4096    sort.bin
root@riscv:~# life.bin
(Runs Conway's Game of Life)
```

## Memory Map

| Address Range | Description | Privilege |
| :--- | :--- | :--- |
| `0x1000_0000` | UART I/O (Byte-wise) | RW |
| `0x8000_0000` | Bootloader Entry (M-Mode) | RX |
| `0x8010_0000` | Kernel Base (S-Mode) | RWX |
| `0x8020_0000` | User Program Load Address | RWX |
| `0x9000_0000` | Virtual Disk (Memory Mapped) | R |
| `0x9000_0FF8` | Virtual Disk Size Register | R |

## Included Demo Programs

The system supports standard C libraries (via `stdio.h`/`stdlib.h` shims) and includes several demos:

*   **life:** Conway's Game of Life visualization.
*   **sand:** Falling sand physics simulation.
*   **maze:** A* pathfinding algorithm solving a generated maze.
*   **mandelbrot:** Fixed-point arithmetic Mandelbrot set renderer.
*   **sort:** Quick Sort and Merge Sort benchmarks.
*   **fib:** Recursive Fibonacci calculation (stress test for stack and branch prediction).

## Statistics

Upon exit (typing `exit` in the shell), the simulator prints detailed execution statistics:

*   **IPC (Instructions Per Cycle)**
*   **Branch Prediction Accuracy** (GShare/BTB performance)
*   **Cache Hit/Miss Rates** (L1, L2, L3)
*   **Pipeline Stalls** (Breakdown of Memory vs Control vs Data hazards)
