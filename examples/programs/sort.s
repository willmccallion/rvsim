.section .text
    .global _start

#------------------------------------------------------------------------------
# PUTCHAR Macro
#------------------------------------------------------------------------------
.macro PUTCHAR reg
    li   t0, 0x10000000                       # UART Base Address
    sb   \reg, 0(t0)
.endm

#------------------------------------------------------------------------------
# Constants
#------------------------------------------------------------------------------
.equ ARRAY_SIZE, 256                          # Increased to 256 items

#------------------------------------------------------------------------------
# _start
#------------------------------------------------------------------------------
_start:
    li    sp, 0x801FF000                      # Initialize Stack Pointer

    # 1. Generate Random Data
    jal ra, generate_random_data

    # 2. Print "Unsorted" Header
    li a0, 'U'
    PUTCHAR a0
    li a0, 'n'
    PUTCHAR a0
    li a0, 's'
    PUTCHAR a0
    li a0, 'o'
    PUTCHAR a0
    li a0, 'r'
    PUTCHAR a0
    li a0, 't'
    PUTCHAR a0
    li a0, 'e'
    PUTCHAR a0
    li a0, 'd'
    PUTCHAR a0
    li a0, ':'
    PUTCHAR a0
    li a0, 10
    PUTCHAR a0

    # 3. Print Unsorted Array
    jal ra, print_array_func
    li a0, 10
    PUTCHAR a0

    # 4. Perform Insertion Sort
    la   s1, my_array                         # s1 = Array Base
    li   s2, ARRAY_SIZE                       # s2 = Length

    li   t0, 1                                # i = 1

outer_loop:
    bge  t0, s2, sort_done                    # if i >= length, break

    # key = arr[i]
    slli t3, t0, 3                            # offset = i * 8
    add  t3, s1, t3                           # addr = base + offset
    ld   t2, 0(t3)                            # key = arr[i]

    addi t1, t0, -1                           # j = i - 1

inner_loop:
    blt  t1, zero, inner_done                 # if j < 0, break

    # load arr[j]
    slli t5, t1, 3                            # offset = j * 8
    add  t5, s1, t5                           # addr = base + offset
    ld   t4, 0(t5)                            # val = arr[j]

    # Compare
    ble  t4, t2, inner_done                   # if arr[j] <= key, break

    # Swap / Shift: arr[j+1] = arr[j]
    sd   t4, 8(t5)

    addi t1, t1, -1                           # j--
    j    inner_loop

inner_done:
    # arr[j+1] = key
    addi t5, t1, 1                            # j + 1
    slli t5, t5, 3                            # (j+1) * 8
    add  t5, s1, t5                           # addr
    sd   t2, 0(t5)                            # store key

    addi t0, t0, 1                            # i++
    j    outer_loop

sort_done:

    # 5. Print "Sorted" Header
    li a0, 'S'
    PUTCHAR a0
    li a0, 'o'
    PUTCHAR a0
    li a0, 'r'
    PUTCHAR a0
    li a0, 't'
    PUTCHAR a0
    li a0, 'e'
    PUTCHAR a0
    li a0, 'd'
    PUTCHAR a0
    li a0, ':'
    PUTCHAR a0
    li a0, 10
    PUTCHAR a0

    # 6. Print Sorted Array
    jal ra, print_array_func
    li a0, 10
    PUTCHAR a0

    # Exit
    li a0, 0
    li a7, 93
    ecall

#------------------------------------------------------------------------------
# Helper Functions
#------------------------------------------------------------------------------
generate_random_data:
    la   t0, my_array
    li   t1, 0
    li   t2, ARRAY_SIZE
    la   t3, seed
    ld   t4, 0(t3)
    li   t5, 6364136223846793005              # LCG Multiplier
    li   t6, 1000                             # Modulo 1000 (0-999)

gen_loop:
    bge  t1, t2, gen_done
    mul  t4, t4, t5
    addi t4, t4, 1
    remu a0, t4, t6                           # a0 = rand % 1000
    sd   a0, 0(t0)
    addi t0, t0, 8
    addi t1, t1, 1
    j    gen_loop
gen_done:
    sd   t4, 0(t3)
    ret

print_array_func:
    addi sp, sp, -16
    sd   ra, 0(sp)
    sd   s0, 8(sp)
    li   s0, 0
    la   t5, my_array
pa_loop:
    li   t6, ARRAY_SIZE
    bge  s0, t6, pa_done
    slli t6, s0, 3
    add  t6, t5, t6
    ld   a0, 0(t6)
    jal  ra, print_decimal
    addi s0, s0, 1
    li   t6, 8
    remu t6, s0, t6
    beq  t6, zero, pa_newline
    li   a0, 32
    PUTCHAR a0
    j    pa_loop
pa_newline:
    li   a0, 10
    PUTCHAR a0
    j    pa_loop
pa_done:
    ld   ra, 0(sp)
    ld   s0, 8(sp)
    addi sp, sp, 16
    ret

print_decimal:
    addi sp, sp, -32
    sd   ra, 0(sp)
    li   t0, 10
    add  t1, sp, 32                           # Buffer end
    add  t2, a0, zero                         # Value
    bne  t2, zero, pd_convert
    addi t1, t1, -1
    li   t3, '0'
    sb   t3, 0(t1)
    j    pd_print
pd_convert:
    remu t3, t2, t0
    divu t2, t2, t0
    addi t3, t3, 48
    addi t1, t1, -1
    sb   t3, 0(t1)
    bne  t2, zero, pd_convert
pd_print:
    add  t2, sp, 32
pd_print_loop:
    beq  t1, t2, pd_done
    lb   a0, 0(t1)
    PUTCHAR a0
    addi t1, t1, 1
    j    pd_print_loop
pd_done:
    ld   ra, 0(sp)
    addi sp, sp, 32
    ret

    .section .data
    .balign 8
seed:
    .dword 9999999

    .section .bss
    .balign 8
my_array:
    .space 2048                               # 256 * 8 bytes

