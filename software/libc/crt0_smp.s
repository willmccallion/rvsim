# Entry point for programs that run on every hart.
#
# The boot ROM starts every hart at _start with a0 = hart id and a1 = the
# device tree, as spike's does. Hart 0 zeroes BSS, counts the cpus in the
# tree and publishes both; the others wait for it. Every hart switches its
# FPU and vector unit on, takes its own stack below _stack_top and calls
# smp_main(hart_id, hart_count). Hart 0's return value is the exit code,
# sent through HTIF once it returns; the other harts park when they return.
.option norvc
.set STACK_SIZE, 0x10000
.section .data
.align 4
smp_bss_ready: .word 0
.align 8
smp_hart_count: .dword 0
.section .text
    .global _start
    .extern smp_main
    .extern fdt_cpu_count
    .extern exit
    .extern _bss_start
    .extern _bss_end
    .extern _stack_top
    .extern __global_pointer$
_start:
.option push
.option norelax
    la gp, __global_pointer$
.option pop
    csrr s0, mhartid           # hart id
    mv s2, a1                  # device tree
    li t0, 0x2200              # mstatus.FS and VS = Initial
    csrs mstatus, t0
    la sp, _stack_top
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
    mv a0, s2
    call fdt_cpu_count
    la t0, smp_hart_count
    sd a0, 0(t0)
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
    la t0, smp_hart_count
    ld s1, 0(t0)
    mv a0, s0
    mv a1, s1
    call smp_main
    bnez s0, park
    call exit
park:
    j park
