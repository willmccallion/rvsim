.option norvc
.section .text
    .global _start
    .extern main

_start:
    # The kernel sets SP to 0x801FF000 before jumping here.
    # We can use it as is, or align it just to be safe.
    andi sp, sp, -16

    # We pass 0 for argc and argv.
    li a0, 0
    li a1, 0
    call main

    # main returns the exit code in a0.
    # The simulator expects exit code in a0 and syscall 93 in a7.
    li a7, 93
    ecall

loop:
    j loop
