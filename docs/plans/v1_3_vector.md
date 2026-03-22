# RVV 1.0 Vector Extension Implementation Plan (v1.3)

## Overview

This document specifies the full RVV 1.0 implementation for the rvsim RISC-V simulator.
The simulator currently supports RV64IMAFDC with both in-order and out-of-order (O3)
backends in a 10-stage pipeline: Fetch1 → Fetch2 → Decode → Rename → Issue → Execute →
Mem1 → Mem2 → Writeback → Commit.

**Target:** Full RVV 1.0 spec compliance with configurable VLEN (128–2048 bits) and a
realistic vector lane execution model.

**Version bump:** v1.2.x → v1.3.0

---

## 1. Architectural Parameters

| Parameter | Range | Default | Notes |
|-----------|-------|---------|-------|
| VLEN | 128–2048 bits, power of 2 | 128 | Vector register width |
| ELEN | 64 | 64 | Maximum element width (fixed for RV64) |
| VLENB | VLEN/8 | 16 | Read-only CSR, vector register bytes |
| Num Lanes | 1–32 | VLEN/64 | Vector execution lanes |
| Vec PRF Size | 48–256 | 64 | O3: physical vector register count |
| Chaining | on/off | on | Vector operation chaining |

---

## 2. Vector CSRs

### 2.1 CSR Addresses

| CSR | Address | R/W | Description |
|-----|---------|-----|-------------|
| vstart | 0x008 | RW | Vector start position |
| vxsat | 0x009 | RW | Fixed-point saturation flag |
| vxrm | 0x00A | RW | Fixed-point rounding mode |
| vcsr | 0x00F | RW | Combined vxsat\|vxrm (vxsat=bit0, vxrm=bits2:1) |
| vl | 0xC20 | RO | Vector length (set by vsetvl family only) |
| vtype | 0xC21 | RO | Vector type (set by vsetvl family only) |
| vlenb | 0xC22 | RO | VLEN/8 (constant) |

### 2.2 vtype Encoding (bits of vtype CSR)

| Bits | Field | Description |
|------|-------|-------------|
| 2:0 | vlmul[2:0] | Vector register group multiplier |
| 5:3 | vsew[2:0] | Selected element width |
| 6 | vta | Tail agnostic (0=undisturbed, 1=agnostic) |
| 7 | vma | Mask agnostic (0=undisturbed, 1=agnostic) |
| XLEN-1 | vill | Illegal value (set if unsupported config) |

### 2.3 VLMUL Encoding

| vlmul[2:0] | LMUL | Register group size |
|-------------|------|---------------------|
| 000 | 1 | 1 |
| 001 | 2 | 2 |
| 010 | 4 | 4 |
| 011 | 8 | 8 |
| 100 | RESERVED | sets vill=1 |
| 101 | 1/8 | 1 (fractional) |
| 110 | 1/4 | 1 (fractional) |
| 111 | 1/2 | 1 (fractional) |

### 2.4 VSEW Encoding

| vsew[2:0] | SEW (bits) |
|-----------|------------|
| 000 | 8 |
| 001 | 16 |
| 010 | 32 |
| 011 | 64 |
| 1xx | RESERVED (sets vill=1) |

### 2.5 mstatus VS Field (bits 10:9)

| VS | Value | Meaning |
|----|-------|---------|
| 00 | Off | Vector unit disabled, traps on vector instructions |
| 01 | Initial | Vector state present but clean |
| 10 | Clean | Vector state not modified since last save |
| 11 | Dirty | Vector state modified |

mstatus.SD (bit 63) is set when any of FS, VS, or XS is Dirty.

### 2.6 MISA V Bit

MISA bit 21 (V) indicates vector extension support. When V is present:
`misa = 0x8000_0000_0034_112D` (adds V=1<<21 to existing RV64IMAFDC).

---

## 3. Newtype Safety Strategy

All domain-specific values use strict newtypes with no implicit conversions.
Defined in `core/units/vpu/newtypes.rs`.

| Type | Inner | Invariant | Purpose |
|------|-------|-----------|---------|
| VRegIdx | u8 | 0–31 | Vector architectural register index |
| Sew | enum | E8/E16/E32/E64 | Selected element width |
| Vlmul | enum | Mf8..M8 | Vector length multiplier |
| LmulGroup | u8 | 1/2/4/8 | Physical register group size |
| Eew | Sew | valid Sew | Effective element width (loads/stores) |
| ElemIdx | usize | 0..VLMAX | Element index within vector register |
| Vl | u64 | any | Current vector length |
| Vlmax | usize | computed | Maximum vector length for config |
| Vlen | usize | power of 2, 128–2048 | Vector register width in bits |
| NumLanes | usize | >= 1 | Number of vector execution lanes |
| Nf | u8 | 1–8 | Segment field count |
| VtypeFields | struct | all fields typed | Parsed vtype CSR |
| TailPolicy | enum | Undisturbed/Agnostic | Tail element policy |
| MaskPolicy | enum | Undisturbed/Agnostic | Masked-off element policy |
| Vxrm | enum | 4 variants | Fixed-point rounding mode |
| VecPhysReg | u16 | valid index | O3 physical vector register |

**Key rules:**
- VRegIdx ≠ RegIdx (cannot accidentally pass GPR index where VPR index expected)
- VecPhysReg ≠ PhysReg (scalar and vector PRFs are separate)
- Vl ≠ Vlmax (prevents mixing current VL with maximum)
- TailPolicy ≠ MaskPolicy ≠ bool (no swap risk at call sites)
- ElemIdx ≠ usize (prevents byte-offset/element-index confusion)

---

## 4. Instruction Encoding

### 4.1 Opcode Space

| Opcode | Binary | Instructions |
|--------|--------|--------------|
| OP-V | 0b1010111 (0x57) | Vector arithmetic |
| OP-LOAD-FP | 0b0000111 (0x07) | Vector loads (shared with FLW/FLD) |
| OP-STORE-FP | 0b0100111 (0x27) | Vector stores (shared with FSW/FSD) |

### 4.2 Vector Arithmetic (OP-V) funct3

| funct3 | Category | Operands |
|--------|----------|----------|
| 000 | OPIVV | Integer vector-vector |
| 001 | OPFVV | FP vector-vector |
| 010 | OPMVV | Mask/reduction vector-vector |
| 011 | OPIVI | Integer vector-immediate |
| 100 | OPIVX | Integer vector-scalar |
| 101 | OPFVF | FP vector-scalar |
| 110 | OPMVX | Mask/move vector-scalar |
| 111 | OPCFG | Configuration (vsetvl family) |

### 4.3 Vector Load/Store Width (funct3 in OP-LOAD-FP/OP-STORE-FP)

| funct3 | Width | Notes |
|--------|-------|-------|
| 000 | EEW=8 | VLE8/VSE8 |
| 010 | 32 | FLW/FSW (scalar FP — existing) |
| 011 | 64 | FLD/FSD (scalar FP — existing) |
| 101 | EEW=16 | VLE16/VSE16 |
| 110 | EEW=32 | VLE32/VSE32 |
| 111 | EEW=64 | VLE64/VSE64 |

### 4.4 Vector Arithmetic Instruction Format

```
31    26 25 24   20 19   15 14 12 11    7 6     0
[funct6] [vm] [vs2/rs2] [vs1/rs1] [funct3] [vd/rd] [opcode]
```

### 4.5 Vector Load/Store Instruction Format

```
31 29 28 27 26 25 24     20 19   15 14 12 11    7 6     0
[nf] [mew] [mop] [vm] [lumop/vs2] [rs1] [width] [vd/vs3] [opcode]
```

Fields:
- `nf[2:0]` (bits 31:29): Number of fields minus 1 (segment loads/stores)
- `mew` (bit 28): Extended memory element width (must be 0 for RVV 1.0)
- `mop[1:0]` (bits 27:26): Memory addressing mode
  - 00 = unit-stride
  - 01 = indexed (unordered)
  - 10 = strided
  - 11 = indexed (ordered)
- `vm` (bit 25): 0=masked, 1=unmasked
- `lumop/sumop` (bits 24:20): Unit-stride sub-variant
  - 00000 = unit-stride
  - 01000 = whole-register
  - 01011 = mask
  - 10000 = fault-only-first (loads only)

---

## 5. Complete Instruction List

### 5.1 Configuration (3 instructions)

| Instruction | Encoding | Description |
|-------------|----------|-------------|
| vsetvli | OP-V, OPCFG, bit31=0 | Set vl from rs1, vtype from zimm[10:0] |
| vsetivli | OP-V, OPCFG, bit31=1,bit30=1 | Set vl from uimm[4:0], vtype from zimm[9:0] |
| vsetvl | OP-V, OPCFG, bit31=1,bit30=0 | Set vl from rs1, vtype from rs2 |

### 5.2 Integer Arithmetic (~50 instructions)

| funct6 | Mnemonic | VV | VX | VI | Description |
|--------|----------|----|----|----|----|
| 000000 | vadd | Y | Y | Y | Addition |
| 000010 | vsub | Y | Y | - | Subtraction |
| 000011 | vrsub | - | Y | Y | Reverse subtract |
| 000100 | vminu | Y | Y | - | Unsigned minimum |
| 000101 | vmin | Y | Y | - | Signed minimum |
| 000110 | vmaxu | Y | Y | - | Unsigned maximum |
| 000111 | vmax | Y | Y | - | Signed maximum |
| 001001 | vand | Y | Y | Y | Bitwise AND |
| 001010 | vor | Y | Y | Y | Bitwise OR |
| 001011 | vxor | Y | Y | Y | Bitwise XOR |
| 001100 | vrgather | Y | Y | Y | Gather (permutation) |
| 001110 | vrgatherei16 | Y | - | - | Gather with 16-bit indices |
| 001110 | vslideup | - | Y | Y | Slide up |
| 001111 | vslidedown | - | Y | Y | Slide down |
| 010000 | vadc | Y | Y | Y | Add with carry (v0) |
| 010001 | vmadc | Y | Y | Y | Carry-out of add |
| 010010 | vsbc | Y | Y | - | Subtract with borrow |
| 010011 | vmsbc | Y | Y | - | Borrow-out of sub |
| 010111 | vmerge/vmv | Y | Y | Y | Conditional merge / unconditional move |
| 011000 | vmseq | Y | Y | Y | Set if equal |
| 011001 | vmsne | Y | Y | Y | Set if not equal |
| 011010 | vmsltu | Y | Y | - | Set if less than (unsigned) |
| 011011 | vmslt | Y | Y | - | Set if less than (signed) |
| 011100 | vmsleu | Y | Y | Y | Set if ≤ (unsigned) |
| 011101 | vmsle | Y | Y | Y | Set if ≤ (signed) |
| 011110 | vmsgtu | - | Y | Y | Set if > (unsigned) |
| 011111 | vmsgt | - | Y | Y | Set if > (signed) |
| 100101 | vsll | Y | Y | Y | Shift left logical |
| 101000 | vsrl | Y | Y | Y | Shift right logical |
| 101001 | vsra | Y | Y | Y | Shift right arithmetic |

### 5.3 Integer Multiply/Divide (~16 instructions)

| funct6 | Mnemonic | Category | Description |
|--------|----------|----------|-------------|
| 100101 | vmul | OPMVV/OPMVX | Multiply low bits |
| 100111 | vmulh | OPMVV/OPMVX | Multiply high (signed) |
| 100100 | vmulhu | OPMVV/OPMVX | Multiply high (unsigned) |
| 100110 | vmulhsu | OPMVV/OPMVX | Multiply high (signed×unsigned) |
| 100000 | vdivu | OPMVV/OPMVX | Unsigned divide |
| 100001 | vdiv | OPMVV/OPMVX | Signed divide |
| 100010 | vremu | OPMVV/OPMVX | Unsigned remainder |
| 100011 | vrem | OPMVV/OPMVX | Signed remainder |
| 101101 | vmacc | OPMVV/OPMVX | Multiply-accumulate (vd += vs1 * vs2) |
| 101111 | vnmsac | OPMVV/OPMVX | Negated multiply-sub (vd -= vs1 * vs2) |
| 101001 | vmadd | OPMVV/OPMVX | Multiply-add (vd = vs1 * vd + vs2) |
| 101011 | vnmsub | OPMVV/OPMVX | Negated multiply-sub variant |

### 5.4 Widening/Narrowing Integer (~20 instructions)

| funct6 | Mnemonic | Description |
|--------|----------|-------------|
| 110000 | vwaddu | Widening unsigned add |
| 110001 | vwadd | Widening signed add |
| 110010 | vwsubu | Widening unsigned sub |
| 110011 | vwsub | Widening signed sub |
| 110100 | vwaddu.w | Widening add (wide + narrow) |
| 110101 | vwadd.w | Widening add (wide + narrow, signed) |
| 110110 | vwsubu.w | Widening sub (wide - narrow) |
| 110111 | vwsub.w | Widening sub (wide - narrow, signed) |
| 111000 | vwmulu | Widening multiply unsigned |
| 111010 | vwmulsu | Widening multiply signed×unsigned |
| 111011 | vwmul | Widening multiply signed |
| 111100 | vwmaccu | Widening multiply-accumulate unsigned |
| 111101 | vwmacc | Widening multiply-accumulate signed |
| 111110 | vwmaccus | Widening multiply-accumulate unsigned×signed |
| 111111 | vwmaccsu | Widening multiply-accumulate signed×unsigned |
| 101100 | vnsrl | Narrowing shift right logical |
| 101101 | vnsra | Narrowing shift right arithmetic |
| 101110 | vnclipu | Narrowing clip unsigned (saturating) |
| 101111 | vnclip | Narrowing clip signed (saturating) |

### 5.5 Fixed-Point (~12 instructions)

| funct6 | Mnemonic | Description |
|--------|----------|-------------|
| 100000 | vsaddu | Saturating add unsigned |
| 100001 | vsadd | Saturating add signed |
| 100010 | vssubu | Saturating sub unsigned |
| 100011 | vssub | Saturating sub signed |
| 001000 | vaaddu | Averaging add unsigned (uses vxrm) |
| 001001 | vaadd | Averaging add signed (uses vxrm) |
| 001010 | vasubu | Averaging sub unsigned (uses vxrm) |
| 001011 | vasub | Averaging sub signed (uses vxrm) |
| 100111 | vsmul | Signed fractional multiply (uses vxrm) |
| 101010 | vssrl | Scaling shift right logical (uses vxrm) |
| 101011 | vssra | Scaling shift right arithmetic (uses vxrm) |

### 5.6 Floating-Point Arithmetic (~40 instructions)

| funct6 | Mnemonic | VV | VF | Description |
|--------|----------|----|----|-------------|
| 000000 | vfadd | Y | Y | FP add |
| 000010 | vfsub | Y | Y | FP sub |
| 000011 | vfrsub | - | Y | FP reverse sub |
| 100100 | vfmul | Y | Y | FP multiply |
| 100000 | vfdiv | Y | Y | FP divide |
| 100001 | vfrdiv | - | Y | FP reverse divide |
| 000100 | vfmin | Y | - | FP minimum |
| 000110 | vfmax | Y | - | FP maximum |
| 001000 | vfsgnj | Y | Y | FP sign injection |
| 001001 | vfsgnjn | Y | Y | FP negated sign injection |
| 001010 | vfsgnjx | Y | Y | FP XOR sign injection |
| 001110 | vfslide1up | - | Y | FP slide up by 1 (vd[0]=f[rs1]) |
| 001111 | vfslide1down | - | Y | FP slide down by 1 |
| 011000 | vmfeq | Y | Y | FP equal (writes mask) |
| 011001 | vmfle | Y | Y | FP ≤ (writes mask) |
| 011010 | vmford | Y | Y | FP ordered (writes mask) |
| 011011 | vmflt | Y | Y | FP < (writes mask) |
| 011100 | vmfne | Y | Y | FP ≠ (writes mask) |
| 011101 | vmfgt | - | Y | FP > (writes mask) |
| 011110 | vmfge | - | Y | FP ≥ (writes mask) |

**FP FMA (funct6 with OPFVV/OPFVF):**
| funct6 | Mnemonic | Description |
|--------|----------|-------------|
| 101001 | vfmadd | FP multiply-add (vd = vs1*vd + vs2) |
| 101011 | vfnmadd | FP negated multiply-add |
| 101101 | vfmacc | FP multiply-accumulate (vd = vs1*vs2 + vd) |
| 101111 | vfnmacc | FP negated multiply-accumulate |
| 101000 | vfmsub | FP multiply-subtract |
| 101010 | vfnmsub | FP negated multiply-subtract |
| 101100 | vfmsac | FP multiply-subtract-accumulate |
| 101110 | vfnmsac | FP negated multiply-sub-accumulate |

**FP Widening:**
| funct6 | Mnemonic | Description |
|--------|----------|-------------|
| 110000 | vfwadd | Widening FP add |
| 110010 | vfwsub | Widening FP sub |
| 110100 | vfwadd.w | Widening FP add (wide + narrow) |
| 110110 | vfwsub.w | Widening FP sub (wide - narrow) |
| 111000 | vfwmul | Widening FP multiply |
| 111100 | vfwmacc | Widening FP multiply-accumulate |
| 111101 | vfwnmacc | Widening FP neg multiply-accumulate |
| 111110 | vfwmsac | Widening FP multiply-sub-accumulate |
| 111111 | vfwnmsac | Widening FP neg multiply-sub-accumulate |

**FP Conversion (encoded in OPFVV with vs1 field specifying variant):**
- vfcvt.xu.f.v, vfcvt.x.f.v, vfcvt.f.xu.v, vfcvt.f.x.v
- vfcvt.rtz.xu.f.v, vfcvt.rtz.x.f.v
- vfwcvt.xu.f.v, vfwcvt.x.f.v, vfwcvt.f.xu.v, vfwcvt.f.x.v, vfwcvt.f.f.v
- vfwcvt.rtz.xu.f.v, vfwcvt.rtz.x.f.v
- vfncvt.xu.f.w, vfncvt.x.f.w, vfncvt.f.xu.w, vfncvt.f.x.w, vfncvt.f.f.w
- vfncvt.rod.f.f.w, vfncvt.rtz.xu.f.w, vfncvt.rtz.x.f.w
- vfsqrt.v, vfrsqrt7.v, vfrec7.v, vfclass.v

### 5.7 Reductions (~16 instructions)

**Integer reductions (OPMVV):**
- vredsum, vredand, vredor, vredxor
- vredminu, vredmin, vredmaxu, vredmax
- vwredsumu, vwredsum (widening)

**FP reductions (OPFVV):**
- vfredusum (unordered), vfredosum (ordered)
- vfredmin, vfredmax
- vfwredusum, vfwredosum (widening)

### 5.8 Mask Operations (~15 instructions)

**Mask-register logical (OPMVV, funct6 with vs1 encoding):**
- vmand.mm, vmnand.mm, vmandn.mm
- vmor.mm, vmnor.mm, vmorn.mm
- vmxor.mm, vmxnor.mm

**Mask scalar results (write to x[rd]):**
- vcpop.m — population count of mask
- vfirst.m — find-first-set (-1 if none)

**Mask-producing:**
- vmsbf.m — set-before-first
- vmsif.m — set-including-first
- vmsof.m — set-only-first

**Other:**
- viota.m — iota (prefix sum of mask bits → vector)
- vid.v — element index (vd[i] = i)

### 5.9 Permutation Operations (~12 instructions)

- vmv.x.s — move vs2[0] to x[rd]
- vmv.s.x — move x[rs1] to vd[0]
- vfmv.f.s — move vs2[0] to f[rd]
- vfmv.s.f — move f[rs1] to vd[0]
- vslideup.vx/vi, vslidedown.vx/vi
- vslide1up.vx, vslide1down.vx
- vfslide1up.vf, vfslide1down.vf
- vrgather.vv/vx/vi, vrgatherei16.vv
- vcompress.vm
- vmv1r.v, vmv2r.v, vmv4r.v, vmv8r.v (whole-register move)

### 5.10 Vector Loads/Stores (~50+ instruction variants)

**Unit-stride:** vle8, vle16, vle32, vle64 / vse8, vse16, vse32, vse64
**Strided:** vlse8–vlse64 / vsse8–vsse64
**Indexed ordered:** vloxei8–vloxei64 / vsoxei8–vsoxei64
**Indexed unordered:** vluxei8–vluxei64 / vsuxei8–vsuxei64
**Mask:** vlm.v / vsm.v
**Whole-register:** vl1re8–vl8re64 / vs1r–vs8r
**Fault-only-first:** vle8ff–vle64ff
**Segment (nf=2..8):** vlseg, vsseg, vlsseg, vssseg, vloxseg, vsoxseg, vluxseg, vsuxseg

---

## 6. Microarchitectural Design

### 6.1 Vector Data Does NOT Flow Through Scalar Latches

Vector results are VLEN-bit wide (up to 2048 bits, or 16384 bits with LMUL=8).
The existing pipeline carries u64 values in all latches. Vector instructions flow
through the scalar pipeline as **ordering tokens** — the actual VLEN-bit data lives
in a separate Vector Register File (VRF) and Vector Physical Register File (Vec PRF
for O3). The ROB entry for a vector op carries only the scalar result (e.g., vsetvl
writes rd) or 0.

### 6.2 Macro-op Vector Memory

Vector loads/stores are single ROB entries. A vector memory sequencer generates
per-element cache probes internally, accumulating total latency. This avoids micro-op
explosion (a LMUL=8, SEW=8, VLEN=2048 load would be 2048 micro-ops otherwise).

### 6.3 Separate Vector PRF (O3)

The O3 backend gets a dedicated VecPhysRegFile with its own free list and rename map.
Vector physical registers are VLEN-bit wide. The scalar PRF is unchanged.

### 6.4 LMUL Register Group Renaming

For LMUL > 1, a register group (e.g., v0-v3 for LMUL=4) is renamed as a unit.
Each LMUL-group operation consumes LMUL consecutive physical vector registers.

### 6.5 vsetvl Serialization

vsetvl/vsetvli/vsetivli are serializing instructions (like CSR writes). They drain
the pipeline before executing because they change SEW, LMUL, and VL.

### 6.6 Lane Execution Model

Each vector FU has N lanes. A vector op on VL elements takes ceil(VL / N) cycles ×
per-element-group latency. Non-pipelined operations (vdiv, vsqrt) block the lane.
Lane count defaults to VLEN / 64.

### 6.7 Chaining

A dependent vector instruction can start consuming results as soon as the first
element-group is ready. Modeled by tracking per-element-group completion cycles.

---

## 7. Implementation Phases

### Phase 1: Foundation — ISA Module, VRF, CSRs, vsetvl, Config **COMPLETED**

**Goal:** Decode vsetvl family, maintain vector CSRs, VRF with element-wise access,
VLEN in config.

**New files:**
- `isa/rvv/mod.rs` — ISA module root
- `isa/rvv/opcodes.rs` — OP_V opcode constant
- `isa/rvv/funct3.rs` — Vector arithmetic funct3 values
- `isa/rvv/funct6.rs` — All ~60 funct6 values
- `isa/rvv/encoding.rs` — Vector-specific bit extraction
- `core/units/vpu/mod.rs` — VPU module root
- `core/units/vpu/newtypes.rs` — All vector newtypes
- `core/units/vpu/types.rs` — VtypeFields parsing, VLMAX computation
- `core/units/vpu/vsetvl.rs` — vsetvl execution logic
- `core/arch/vpr.rs` — Architectural vector register file

**Modified files:**
- `isa/mod.rs` — add `pub mod rvv`
- `core/arch/mod.rs` — add `pub mod vpr`
- `core/arch/csr.rs` — vector CSR addresses, VS field, MISA V bit
- `core/units/mod.rs` — add `pub mod vpu`
- `core/pipeline/signals.rs` — VectorOp enum, ControlSignals fields
- `core/pipeline/frontend/decode.rs` — vsetvl decode
- `config.rs` — VLEN, num_vec_lanes config

### Phase 2: Vector Integer Arithmetic + Pipeline Integration

**Goal:** All vector integer arithmetic, comparison, fixed-point. Basic timing.

**New files:**
- `core/units/vpu/alu.rs` — Integer ALU (add, sub, logic, shift, compare, merge)
- `core/units/vpu/multiply.rs` — Integer multiply and multiply-accumulate
- `core/units/vpu/divide.rs` — Integer divide/remainder
- `core/units/vpu/widening.rs` — Widening/narrowing operations
- `core/units/vpu/fixed_point.rs` — Saturating/averaging/scaling ops

**Modified files:**
- `core/pipeline/signals.rs` — expand VectorOp to ~80 integer variants
- `core/pipeline/frontend/decode.rs` — full OP-V integer decode
- `core/pipeline/backend/o3/fu_pool.rs` — vector FU types
- Execute stages (both backends) — vector dispatch

### Phase 3: Vector Loads/Stores

**Goal:** All vector memory operations.

**New files:**
- `core/units/vpu/mem.rs` — Vector memory sequencer
- `isa/rvv/mem_encoding.rs` — Load/store encoding decode

### Phase 4: Vector FP, Reductions, Mask Ops, Permutations

**Goal:** Complete RVV 1.0 instruction coverage.

**New files:**
- `core/units/vpu/fpu.rs` — Vector FP operations
- `core/units/vpu/reduction.rs` — Integer and FP reductions
- `core/units/vpu/mask.rs` — Mask logical and scalar ops
- `core/units/vpu/permute.rs` — Slides, gathers, compress

### Phase 5: Vector Lane Model + O3 Pipeline Integration

**Goal:** Cycle-accurate timing, vector PRF, renaming, chaining.

**New files:**
- `core/pipeline/vec_prf.rs` — Vector physical register file
- `core/pipeline/vec_free_list.rs` — Vector register free list
- `core/units/vpu/lane_model.rs` — Lane execution model
- `core/units/vpu/chaining.rs` — Chaining logic

### Phase 6: Disassembler, Testing, Polish

**Goal:** Disassembly, comprehensive tests, edge cases.

---

## 8. vsetvl Specification

Per V 1.0 Section 6.1:

1. If vtype is illegal → set vtype.vill=1, vl=0
2. If rd=x0 && rs1=x0 → keep current vl, change vtype only
3. If rs1=x0 && rd≠x0 → set vl=VLMAX
4. Otherwise:
   - AVL ≤ VLMAX → vl = AVL
   - AVL > VLMAX && AVL < 2×VLMAX → ceil(AVL/2) ≤ vl ≤ VLMAX
   - AVL ≥ 2×VLMAX → vl = VLMAX
   - **Our implementation:** vl = min(AVL, VLMAX)

VLMAX = (VLEN / SEW) × LMUL

---

## 9. Tail/Mask Handling

**Tail elements** (index >= vl):
- vta=Undisturbed: preserve existing value
- vta=Agnostic: write all-1s (our choice — deterministic)

**Masked-off elements** (v0[i]=0 when vm=0):
- vma=Undisturbed: preserve existing value
- vma=Agnostic: write all-1s (our choice — deterministic)

**Body elements** (index < vl, active mask bit):
- Always computed normally

---

## 10. Division by Zero and Overflow

Per RVV 1.0 spec:
- Unsigned divide by zero: quotient = all-1s (2^SEW - 1)
- Signed divide by zero: quotient = all-1s (-1)
- Unsigned remainder by zero: remainder = dividend
- Signed remainder by zero: remainder = dividend
- Signed overflow (MIN_INT / -1): quotient = MIN_INT, remainder = 0

---

## 11. Configuration Parameters

Added to `PipelineConfig`:

```
vlen: usize                    — default 128
num_vec_lanes: usize           — default vlen/64, min 1
prf_vpr_size: usize            — default 64 (O3 only)
vec_chaining: bool             — default true
```

Added to `FuConfig`:

```
num_vec_int_alu: usize         — default 1
vec_int_alu_latency: u64       — default 1
num_vec_int_mul: usize         — default 1
vec_int_mul_latency: u64       — default 3
num_vec_int_div: usize         — default 1
vec_int_div_latency: u64       — default 35
num_vec_fp_add: usize          — default 1
vec_fp_add_latency: u64        — default 4
num_vec_fp_mul: usize          — default 1
vec_fp_mul_latency: u64        — default 5
num_vec_fp_fma: usize          — default 1
vec_fp_fma_latency: u64        — default 5
num_vec_fp_div_sqrt: usize     — default 1
vec_fp_div_sqrt_latency: u64   — default 21
num_vec_mem: usize             — default 1
vec_mem_latency: u64           — default 1
num_vec_permute: usize         — default 1
vec_permute_latency: u64       — default 1
```

---

## 12. File Map Summary

### New Files (26)
```
docs/plans/v1_3_vector.md                        — this document
isa/rvv/mod.rs
isa/rvv/opcodes.rs
isa/rvv/funct3.rs
isa/rvv/funct6.rs
isa/rvv/encoding.rs
isa/rvv/mem_encoding.rs
core/arch/vpr.rs
core/units/vpu/mod.rs
core/units/vpu/newtypes.rs
core/units/vpu/types.rs
core/units/vpu/vsetvl.rs
core/units/vpu/alu.rs
core/units/vpu/multiply.rs
core/units/vpu/divide.rs
core/units/vpu/widening.rs
core/units/vpu/fixed_point.rs
core/units/vpu/fpu.rs
core/units/vpu/reduction.rs
core/units/vpu/mask.rs
core/units/vpu/permute.rs
core/units/vpu/mem.rs
core/pipeline/vec_prf.rs
core/pipeline/vec_free_list.rs
core/units/vpu/lane_model.rs
core/units/vpu/chaining.rs
```

### Modified Files (19)
```
isa/mod.rs
core/arch/csr.rs
core/arch/mod.rs
core/units/mod.rs
core/pipeline/signals.rs
core/pipeline/latches.rs
core/pipeline/frontend/decode.rs
core/pipeline/frontend/rename.rs
core/pipeline/backend/o3/fu_pool.rs
core/pipeline/backend/o3/issue_queue.rs
core/pipeline/backend/o3/execute.rs
core/pipeline/backend/o3/mod.rs
core/pipeline/backend/inorder/execute.rs
core/pipeline/backend/shared/commit.rs
core/pipeline/rob.rs
core/pipeline/scoreboard.rs
core/pipeline/rename_map.rs
core/pipeline/checkpoint.rs
config.rs
isa/disasm.rs
```
