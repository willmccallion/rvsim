.option norvc
.section .text.entry
.global _start
.global switch_to_user

_start:
    # Set up Kernel Stack
    li sp, 0x800FF000
    
    # Initialize sscratch to 0. 
    csrw sscratch, zero

    # Jump to C function kmain
    call kmain

    # If kmain returns, hang
hang:
    j hang

switch_to_user:
    # Save Kernel Callee-Saved Registers to Stack
    addi sp, sp, -112
    
    sd ra, 0(sp)
    sd s0, 8(sp)
    sd s1, 16(sp)
    sd s2, 24(sp)
    sd s3, 32(sp)
    sd s4, 40(sp)
    sd s5, 48(sp)
    sd s6, 56(sp)
    sd s7, 64(sp)
    sd s8, 72(sp)
    sd s9, 80(sp)
    sd s10, 88(sp)
    sd s11, 96(sp)

    # Set Trap Vector to our handler
    la t0, trap_entry
    csrw stvec, t0

    # Save Kernel Stack Pointer to SSCRATCH
    csrw sscratch, sp

    # Setup User Entry
    csrw sepc, a0        # Set jump target

    # Setup Dedicated User Stack
    li sp, 0x80800000

    # Configure Status
    li t0, 0x100
    csrc sstatus, t0

    sret

# ----------------------------------------------------------------
# Trap Handler
# ----------------------------------------------------------------
.align 4
trap_entry:
    # Swap SP with SSCRATCH
    csrrw sp, sscratch, sp

    # Check if we came from Kernel or User
    bnez sp, from_user

from_kernel:
    # We came from Kernel. Restore SP
    csrrw sp, sscratch, sp
    # We are already on the Kernel stack, so just proceed.
    j trap_body

from_user:
    # We came from User. SP is now the Kernel Stack. 
    
trap_body:
    # Check Cause
    csrr t0, scause
    li t1, 8             # 8 = User ECALL
    beq t0, t1, handle_exit

    # If it's not an exit, return scause
    mv a0, t0
    j restore_kernel

handle_exit:
    # The user program put the exit code in a0.
    # It is already there, so we just return it to C.

restore_kernel:
    # Set sscratch to 0 so future traps know we are in Kernel.
    csrw sscratch, zero

    # Restore Kernel Registers
    ld ra, 0(sp)
    ld s0, 8(sp)
    ld s1, 16(sp)
    ld s2, 24(sp)
    ld s3, 32(sp)
    ld s4, 40(sp)
    ld s5, 48(sp)
    ld s6, 56(sp)
    ld s7, 64(sp)
    ld s8, 72(sp)
    ld s9, 80(sp)
    ld s10, 88(sp)
    ld s11, 96(sp)
    addi sp, sp, 112

    ret
