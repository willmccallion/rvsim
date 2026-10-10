# Start-up for the RTL comparison's programs: software/libc/crt0.s with what
# a core needs that rvsim's direct mode provides on its own. The stack
# pointer is set here (Chipyard's boot ROM leaves it undefined), the FPU is
# switched on (Rocket and BOOM reset with mstatus.FS off, rvsim with it on),
# and the exit goes through HTIF's tohost, which rvsim's HTIF device and
# Chipyard's test harness both serve, instead of an exit syscall.
.option norvc
.section .text
    .global _start
    .extern main
    .extern _bss_start
    .extern _bss_end
    .extern _stack_top
    .extern __global_pointer$

_start:
.option push
.option norelax
    la gp, __global_pointer$
.option pop
    la sp, _stack_top

    li t0, 0x2000               # mstatus.FS = Initial
    csrs mstatus, t0

    la t0, _bss_start
    la t1, _bss_end
    bge t0, t1, bss_clear_done
bss_clear_loop:
    sd zero, 0(t0)
    addi t0, t0, 8
    blt t0, t1, bss_clear_loop
bss_clear_done:

    li a0, 0
    li a1, 0
    call main

    # tohost = (code << 1) | 1: the HTIF exit with main's return value.
    slli a0, a0, 1
    ori a0, a0, 1
    la t0, tohost
    sd a0, 0(t0)
exit_loop:
    j exit_loop

.section .tohost, "aw", @progbits
.align 6
    .global tohost
tohost: .dword 0
.align 6
    .global fromhost
fromhost: .dword 0
