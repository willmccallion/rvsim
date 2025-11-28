.option norvc
    .section .text
    .global _start

#------------------------------------------------------------------------------
# _start
#
# Description:
#   The Bootloader Entry Point.
#    Copies the kernel image from the virtual disk to RAM.
#    Configures privilege delegation (M-mode to S-mode).
#    Sets up mstatus to switch to Supervisor mode.
#    Sets mepc to the kernel entry point.
#    Executes mret to jump to the kernel.
#
# Args:
#   None
#
# Returns:
#   a0: The size of the kernel (4096 bytes) passed to the kernel.
#
# Register Usage:
#   s0: Disk source pointer (0x90000000)
#   s1: RAM destination pointer (0x80100000)
#   s2: Byte count for copy loop
#   t0: Temporary for address construction and CSR manipulation
#------------------------------------------------------------------------------
_start:
    # Constructed from shifted constants for readability and consistency.
    li s1, 1024                               # s1 <- 1024
    addi s1, s1, 1025                         # s1 <- 2049 (0x801)
    slli s1, s1, 20                           # s1 <- 0x80100000 (Kernel RAM Base)

    li s0, 9                                  # s0 <- 9
    slli s0, s0, 28                           # s0 <- 0x90000000 (Disk Base)

    # Number of bytes to copy from disk to RAM.
    li s2, 16384                              # s2 <- 4096 (Kernel Size)

copy_loop:
    # Simple byte-wise copy loop. Keeps boot code small and reliable.
    beq s2, zero, copy_done                   # if s2 == 0 goto copy_done
    lbu t0, 0(s0)                             # t0 <- Byte from disk
    sb t0, 0(s1)                              # Store byte to RAM
    addi s0, s0, 1                            # s0 <- s0 + 1 (Next disk addr)
    addi s1, s1, 1                            # s1 <- s1 + 1 (Next RAM addr)
    addi s2, s2, -1                           # s2 <- s2 - 1 (Decrement count)
    j copy_loop                               # goto copy_loop

copy_done:
    # mmeleg/mideleg = -1 delegates all to S-mode so kernel handles them.
    li t0, -1                                 # t0 <- -1 (All bits set)
    csrrw zero, medeleg, t0                   # Delegate exceptions to S-mode
    csrrw zero, mideleg, t0                   # Delegate interrupts to S-mode

    # Clear MPP then set MPP = S-Mode so mret goes to S-mode.
    li t0, 0x1800                             # t0 <- MPP Mask (bits 11 and 12)
    csrrc zero, mstatus, t0                   # Clear MPP bits in mstatus
    li t0, 0x800                              # t0 <- 0x800 (Supervisor Mode bit)
    csrrs zero, mstatus, t0                   # Set MPP to Supervisor

    li t0, 1024                               # t0 <- 1024
    addi t0, t0, 1025                         # t0 <- 2049
    slli t0, t0, 20                           # t0 <- 0x80100000 (Kernel Entry)
    csrrw zero, mepc, t0                      # mepc <- Kernel Entry Address

    # Put 4096 into a0 so the kernel knows the correct size.
    lui a0, 1                                 # a0 <- 4096 (0x1000)
    mret                                      # Return from exception (Jump to Kernel)

hang:
    # Safety fallback. Execution should never reach here.
    j hang                                    # Infinite loop
