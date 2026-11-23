# The comparison programs' start-up, the same instructions on both
# simulators: the loader leaves the stack pointer, the FPU and the vector
# unit set up (gem5's SE process loader, rvsim's run_rvsim.py), and the
# program ends with the m5 exit pseudo-instruction, which gem5 takes in SE
# mode and rvsim bare-metal.
.option norvc
.section .text
    .global _start
    .extern main

_start:
.option push
.option norelax
    la gp, __global_pointer$
.option pop
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
    .word 0x4200007b            # m5_exit
spin:
    j spin
