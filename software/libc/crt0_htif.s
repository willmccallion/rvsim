.option norvc
.section .text
    .global _start
    .extern main
    .extern _bss_start
    .extern _bss_end
    .extern __global_pointer$
    .extern stdio_flush
    .extern htif_exit
    .extern _stack_top

# The HTIF start-up: as crt0.s, but with what a core other than rvsim's
# direct mode leaves to the program (the stack pointer, the FPU and the
# vector unit switched on), the output flushed to the host, and the exit
# through tohost, so the same ELF runs on rvsim, spike and Chipyard's
# Verilator harness.
_start:
.option push
.option norelax
    la gp, __global_pointer$
.option pop
    la sp, _stack_top
    li t0, 0x2200               # mstatus.FS and VS = Initial
    csrs mstatus, t0

    la t0, _bss_start
    la t1, _bss_end
    bge t0, t1, bss_clear_done
bss_clear_loop:
    sd zero, 0(t0)
    addi t0, t0, 8
    blt t0, t1, bss_clear_loop
bss_clear_done:

    andi sp, sp, -16
    li a0, 0
    li a1, 0
    call main

    mv s0, a0
    call stdio_flush
    mv a0, s0
    call htif_exit
