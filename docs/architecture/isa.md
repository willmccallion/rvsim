# ISA

rvsim implements **RV64IMAFDC** with the full privileged architecture specification.

## Extensions

### RV64I — Base Integer

Full 64-bit integer instruction set including all W-variants (32-bit operations with sign extension to 64 bits):

- Arithmetic: `ADD`, `SUB`, `ADDI`, `ADDW`, `SUBW`, etc.
- Logic: `AND`, `OR`, `XOR`, `ANDI`, `ORI`, `XORI`
- Shifts: `SLL`, `SRL`, `SRA`, `SLLI`, `SRLI`, `SRAI` (+ W variants)
- Comparison: `SLT`, `SLTU`, `SLTI`, `SLTIU`
- Loads/stores: `LB`, `LH`, `LW`, `LD`, `LBU`, `LHU`, `LWU`, `SB`, `SH`, `SW`, `SD`
- Branches: `BEQ`, `BNE`, `BLT`, `BGE`, `BLTU`, `BGEU`
- Jumps: `JAL`, `JALR`
- Upper immediate: `LUI`, `AUIPC`
- System: `ECALL`, `EBREAK`, `FENCE`, `FENCE.I`
- CSR access: `CSRRW`, `CSRRS`, `CSRRC`, `CSRRWI`, `CSRRSI`, `CSRRCI`

### M — Multiply/Divide

- Multiply: `MUL`, `MULH`, `MULHSU`, `MULHU`, `MULW`
- Divide: `DIV`, `DIVU`, `REM`, `REMU`, `DIVW`, `DIVUW`, `REMW`, `REMUW`

Division by zero and overflow follow the RISC-V specification (no exceptions, defined results).

### A — Atomics

**Load-reserved / Store-conditional:**

- `LR.W`, `LR.D` — load and set reservation
- `SC.W`, `SC.D` — conditional store (succeeds only if reservation is still valid)

LR/SC includes a forward progress guarantee: the implementation ensures SC will eventually succeed if the reservation is not broken by another store to the same address.

**Atomic memory operations (AMO):**

`AMOSWAP`, `AMOADD`, `AMOAND`, `AMOOR`, `AMOXOR`, `AMOMIN`, `AMOMAX`, `AMOMINU`, `AMOMAXU` — for both word (.W) and doubleword (.D).

### F — Single-Precision Float

IEEE 754 single-precision floating point:

- Arithmetic: `FADD.S`, `FSUB.S`, `FMUL.S`, `FDIV.S`, `FSQRT.S`
- Fused multiply-add: `FMADD.S`, `FMSUB.S`, `FNMADD.S`, `FNMSUB.S`
- Comparison: `FEQ.S`, `FLT.S`, `FLE.S`, `FMIN.S`, `FMAX.S`
- Conversion: `FCVT.W.S`, `FCVT.WU.S`, `FCVT.L.S`, `FCVT.LU.S`, `FCVT.S.W`, `FCVT.S.WU`, `FCVT.S.L`, `FCVT.S.LU`
- Sign injection: `FSGNJ.S`, `FSGNJN.S`, `FSGNJX.S`
- Classification: `FCLASS.S`
- Move: `FMV.X.W`, `FMV.W.X`

**NaN-boxing** is enforced per spec section 12.2: single-precision values stored in 64-bit FP registers must have all upper bits set to 1. On load, values that fail the NaN-boxing check are replaced with the canonical NaN.

### D — Double-Precision Float

Full parity with the F extension for 64-bit double-precision values. Includes all arithmetic, FMA, comparison, conversion, and classification instructions with `.D` suffix. Also includes `FCVT.S.D` and `FCVT.D.S` for float-double conversion.

### C — Compressed Instructions

16-bit compressed instruction encoding. All compressed instructions are expanded to their 32-bit equivalents at decode time (Fetch2 stage). The fetch unit handles mixed 16/32-bit instruction streams, including instructions that span cache line boundaries.

## Privileged Architecture

### Privilege Modes

Three privilege levels: Machine (M), Supervisor (S), and User (U). The simulator starts in M-mode and transitions between modes via traps and return instructions.

### CSR Set

Full implementation of the standard CSRs:

| Category | CSRs |
|----------|------|
| **Machine** | `mstatus`, `misa`, `medeleg`, `mideleg`, `mie`, `mtvec`, `mscratch`, `mepc`, `mcause`, `mtval`, `mip`, `mcounteren`, `mcountinhibit` |
| **Supervisor** | `sstatus`, `sie`, `stvec`, `sscratch`, `sepc`, `scause`, `stval`, `sip`, `satp`, `scounteren`, `stimecmp` |
| **Counters** | `cycle`, `time`, `instret`, `mcycle`, `minstret` |
| **FP** | `fflags`, `frm`, `fcsr` |
| **PMP** | `pmpcfg0`–`pmpcfg3`, `pmpaddr0`–`pmpaddr15` |

### Trap Handling

- **Trap delegation**: `medeleg` and `mideleg` configure which exceptions and interrupts are delegated from M-mode to S-mode
- **MRET / SRET**: return from trap, restoring privilege level and interrupt state
- **WFI**: wait for interrupt (halts pipeline, increments WFI cycle counter)

### Virtual Memory

SV39 translation controlled by `satp` CSR. Writing to `satp` triggers a pipeline drain and TLB flush (deferred to commit after store buffer drains).

`SFENCE.VMA` supports:

- Global flush (no arguments)
- Address-specific flush (`rs1` specifies virtual address)
- ASID-specific flush (`rs2` specifies ASID)

### Physical Memory Protection (PMP)

16 PMP regions with three address matching modes:

- **TOR** (Top of Range)
- **NAPOT** (Naturally Aligned Power of Two)
- **NA4** (Naturally Aligned 4-byte)

PMP is checked on every memory access in S-mode and U-mode. M-mode accesses bypass PMP unless `mstatus.MPRV` is set.

### Privileged Control Bits

Key `mstatus` fields:

- **TSR** (Trap SRET): traps SRET in S-mode
- **TW** (Timeout Wait): traps WFI in S-mode after timeout
- **TVM** (Trap Virtual Memory): traps `satp` access and `SFENCE.VMA` in S-mode
- **FS** (FP State): tracks whether FP state is clean/dirty; traps FP instructions when FS=Off

## Test Compliance

Passes all **134/134** tests in [`riscv-software-src/riscv-tests`](https://github.com/riscv-software-src/riscv-tests):

- `rv64ui` — base integer instructions
- `rv64um` — multiply/divide
- `rv64ua` — atomics (LR/SC, AMO)
- `rv64uf` — single-precision float
- `rv64ud` — double-precision float
- `rv64uc` — compressed instructions
- `rv64mi` — machine-mode traps and CSRs
- `rv64si` — supervisor-mode traps and virtual memory
