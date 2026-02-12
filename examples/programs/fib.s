.section .text.entry
    .global _start

#------------------------------------------------------------------------------
# PUTCHAR
#
# Description:
#   Macro to write one byte to the UART address.
#
# Args:
#   reg: The register containing the byte to write
#
# Register Usage:
#   t0: UART Base Address
#------------------------------------------------------------------------------
.macro PUTCHAR reg
    li   t0, 0x10000000                       # t0 <- UART Base Address
    sb   \reg, 0(t0)                          # Store byte to UART
.endm

#------------------------------------------------------------------------------
# _start
#
# Description:
#   Entry point of the program. Calculates the 10th Fibonacci number and prints
#   the result in the format "fib(10)=55".
#
# Register Usage:
#   s0: Saves the result of fib(n)
#   s1: Saves the input n
#------------------------------------------------------------------------------
_start:
    li    sp, 0x00000000801FF000              # Initialize Stack Pointer

    # n value to calculate
    li a0, 20                                 # a0 <- 20
    add s1, a0, zero                          # s1 <- a0 (save n for printing)
    jal ra, fib                               # Call fib(20)
    add s0, a0, zero                          # s0 <- a0 (save result)

    # print "fib("
    li a0, 'f'                                # a0 <- 'f'
    PUTCHAR a0                                # Print char
    li a0, 'i'                                # a0 <- 'i'
    PUTCHAR a0                                # Print char
    li a0, 'b'                                # a0 <- 'b'
    PUTCHAR a0                                # Print char
    li a0, '('                                # a0 <- '('
    PUTCHAR a0                                # Print char

    # print n
    add a0, s1, zero                          # a0 <- s1 (n)
    jal ra, print_decimal                     # Print n

    # print ")="
    li a0, ')'                                # a0 <- ')'
    PUTCHAR a0                                # Print char
    li a0, '='                                # a0 <- '='
    PUTCHAR a0                                # Print char

    # print fib(n)
    add a0, s0, zero                          # a0 <- s0 (result)
    jal ra, print_decimal                     # Print result

    # print "\n"
    li a0, 10                                 # a0 <- Newline
    PUTCHAR a0                                # Print char

    # exit(0)
    li a0, 0                                  # a0 <- 0
    li a7, 93                                 # a7 <- 93 (Exit syscall)
    ecall

#------------------------------------------------------------------------------
# fib
#
# Description:
#   Recursively calculates the nth Fibonacci number.
#   Base case: if n < 2, return n.
#   Recursive step: return fib(n-1) + fib(n-2).
#
# Args:
#   a0: The integer n
#
# Returns:
#   a0: The nth Fibonacci number
#
# Register Usage:
#   s0: Saves n
#   s1: Saves result of fib(n-1)
#   t0: Base case comparison value (2)
#------------------------------------------------------------------------------
fib:
    li t0, 2                                  # t0 <- 2
    blt a0, t0, fib_base_return               # if n < 2 goto fib_base_return

    addi sp, sp, -24                          # Make room on stack
    sd s0, 0(sp)                              # Save s0 on stack
    sd s1, 8(sp)                              # Save s1 on stack
    sd ra, 16(sp)                             # Save ra on stack

    add s0, a0, zero                          # s0 <- n

    addi a0, s0, -1                           # a0 <- n - 1
    jal ra, fib                               # Call fib(n-1)
    add s1, a0, zero                          # s1 <- Result of fib(n-1)

    addi a0, s0, -2                           # a0 <- n - 2
    jal ra, fib                               # Call fib(n-2)

    add a0, s1, a0                            # a0 <- fib(n-1) + fib(n-2)

    ld s0, 0(sp)                              # Restore s0 from stack
    ld s1, 8(sp)                              # Restore s1 from stack
    ld ra, 16(sp)                             # Restore ra from stack
    addi sp, sp, 24                           # Restore stack

fib_base_return:
    ret

#------------------------------------------------------------------------------
# print_decimal
#
# Description:
#   Prints the unsigned integer in a0 to the UART.
#   Converts integer to ASCII string in a local stack buffer, then prints.
#
# Args:
#   a0: The unsigned integer to print
#
# Register Usage:
#   s0: Value to print
#   s1: Length of string
#   t1: Divisor (10)
#   t2: Remainder / Character
#   t3: Pointer to buffer
#------------------------------------------------------------------------------
print_decimal:
    addi sp, sp, -96                          # Make room on stack
    sd   ra, 0(sp)                            # Save ra on stack
    sd   s0, 8(sp)                            # Save s0 on stack
    sd   s1, 16(sp)                           # Save s1 on stack
    sd   s2, 24(sp)                           # Save s2 on stack
    sd   t0, 32(sp)                           # Save t0 on stack
    sd   t1, 40(sp)                           # Save t1 on stack
    sd   t2, 48(sp)                           # Save t2 on stack
    sd   t3, 56(sp)                           # Save t3 on stack

    addi t3, sp, 96                           # t3 <- End of frame
    addi t3, t3, -32                          # t3 <- Buffer end

    add  s0, a0, zero                         # s0 <- Value
    li   s1, 0                                # s1 <- Length

    bne  s0, zero, pd_convert                 # if value != 0 goto pd_convert

    # Handle zero case
    addi t3, t3, -1                           # Decrement buffer pointer
    li   t2, '0'                              # t2 <- '0'
    sb   t2, 0(t3)                            # Store '0' in buffer
    li   s1, 1                                # s1 <- 1
    j    pd_print                             # goto pd_print

pd_convert:
    li   t1, 10                               # t1 <- 10
pd_conv_loop:
    rem  t2, s0, t1                           # t2 <- s0 % 10
    div  s0, s0, t1                           # s0 <- s0 / 10
    addi t2, t2, 48                           # t2 <- Convert to ASCII
    addi t3, t3, -1                           # Decrement buffer pointer
    sb   t2, 0(t3)                            # Store char in buffer
    addi s1, s1, 1                            # s1 <- Length + 1
    bne  s0, zero, pd_conv_loop               # if s0 != 0 goto pd_conv_loop

pd_print:
    beq  s1, zero, pd_done                    # if length == 0 goto pd_done
pd_print_loop:
    lb   a0, 0(t3)                            # a0 <- Char from buffer
    PUTCHAR a0                                # Print char
    addi t3, t3, 1                            # Increment buffer pointer
    addi s1, s1, -1                           # Decrement length
    bne  s1, zero, pd_print_loop              # if length != 0 goto pd_print_loop

pd_done:
    ld   ra, 0(sp)                            # Restore ra from stack
    ld   s0, 8(sp)                            # Restore s0 from stack
    ld   s1, 16(sp)                           # Restore s1 from stack
    ld   s2, 24(sp)                           # Restore s2 from stack
    ld   t0, 32(sp)                           # Restore t0 from stack
    ld   t1, 40(sp)                           # Restore t1 from stack
    ld   t2, 48(sp)                           # Restore t2 from stack
    ld   t3, 56(sp)                           # Restore t3 from stack
    addi sp, sp, 96                           # Restore stack
    ret

