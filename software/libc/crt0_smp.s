# Entry point for programs that run on every hart.
#
# The simulator starts every hart at _start with a1 = hart count (and
# a0 = hart id, which is also mhartid). Hart 0 zeroes BSS and publishes a
# flag; the others wait for it. Every hart gets its own stack below the
# simulator-provided one and then calls smp_main(hart_id, hart_count).
# Hart 0's return value is the exit code; the other harts park when they
# return.
.option norvc
.set STACK_SIZE, 0x10000

.section .data
.align 4
smp_bss_ready: .word 0

.section .text
    .global _start
    .extern smp_main
    .extern _bss_start
    .extern _bss_end
    .extern __global_pointer$

_start:
.option push
.option norelax
    la gp, __global_pointer$
.option pop
    csrr s0, mhartid           # hart id
    mv s1, a1                  # hart count

    andi sp, sp, -16
    li t0, STACK_SIZE
    mul t1, s0, t0
    sub sp, sp, t1

    bnez s0, wait_bss

    la t0, _bss_start
    la t1, _bss_end
    bge t0, t1, bss_clear_done
bss_clear_loop:
    sd zero, 0(t0)
    addi t0, t0, 8
    blt t0, t1, bss_clear_loop
bss_clear_done:
    fence rw, rw
    la t0, smp_bss_ready
    li t1, 1
    sw t1, 0(t0)
    j run

wait_bss:
    la t0, smp_bss_ready
wait_bss_loop:
    lw t1, 0(t0)
    beqz t1, wait_bss_loop
    fence r, rw

run:
    mv a0, s0
    mv a1, s1
    call smp_main

    bnez s0, park
    li a7, 93
    ecall

park:
    j park
