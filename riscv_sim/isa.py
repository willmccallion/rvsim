"""
RISC-V Application Binary Interface (ABI) and CSR address definitions.

This module provides:
1. **General-purpose register indices:** ABI names (e.g., REG_RA, REG_SP, REG_A0) for syscalls and calling convention.
2. **Control and Status Register addresses:** Supervisor and machine CSRs (SSTATUS, MIE, MTVEC, etc.) for scripting.
"""

REG_ZERO = 0
REG_RA   = 1
REG_SP   = 2
REG_GP   = 3
REG_TP   = 4
REG_T0   = 5
REG_T1   = 6
REG_T2   = 7
REG_S0   = 8
REG_FP   = 8
REG_S1   = 9
REG_A0   = 10
REG_A1   = 11
REG_A2   = 12
REG_A3   = 13
REG_A4   = 14
REG_A5   = 15
REG_A6   = 16
REG_A7   = 17
REG_S2   = 18
REG_S3   = 19
REG_S4   = 20
REG_S5   = 21
REG_S6   = 22
REG_S7   = 23
REG_S8   = 24
REG_S9   = 25
REG_S10  = 26
REG_S11  = 27
REG_T3   = 28
REG_T4   = 29
REG_T5   = 30
REG_T6   = 31

CSR_SSTATUS = 0x100
CSR_SIE     = 0x104
CSR_STVEC   = 0x105
CSR_SSCRATCH= 0x140
CSR_SEPC    = 0x141
CSR_SCAUSE  = 0x142
CSR_STVAL   = 0x143
CSR_SIP     = 0x144
CSR_SATP    = 0x180

CSR_MSTATUS = 0x300
CSR_MISA    = 0x301
CSR_MEDELEG = 0x302
CSR_MIDELEG = 0x303
CSR_MIE     = 0x304
CSR_MTVEC   = 0x305
CSR_MEPC    = 0x341
CSR_MCAUSE  = 0x342
CSR_MTVAL   = 0x343
CSR_MIP     = 0x344
