#!/usr/bin/env python3
"""Generate random self-checking RVV vector torture tests.

Each generated test:
  1. Configures vtype (SEW/LMUL) with vsetvli
  2. Initializes vector operands with known scalar values
  3. Performs vector operations under test
  4. Extracts results to scalars (vmv.x.s / vfmv.f.s)
  5. Computes expected values via scalar instructions
  6. Self-checks via bne → fail

Usage:
    python generate_vec.py [--seed SEED] [--count N] [--length L] [--outdir DIR]
"""

import argparse
import os
import random
import struct
import sys

# ── Register pools ──────────────────────────────────────────────────────────

SCALAR_REGS = list(range(5, 32))  # x5-x31 (same as scalar torture)
MEM_BASE_REG = 4                  # x4/tp → scratch memory base
TESTNUM_REG = 3                   # x3/gp = test case number

# Vector register allocation:
# v0        = mask register (reserved by spec for masking)
# v1-v7     = scratch / temporaries
# v8-v31    = primary operand pool
VREG_SCRATCH = list(range(1, 8))
VREG_POOL_ALL = list(range(8, 32))

# ── Valid SEW/LMUL configurations ───────────────────────────────────────────
# (sew_str, lmul_str, sew_bits, approx_vlmax for VLEN=128)
VTYPE_CONFIGS = [
    ("e8",  "m1",  8,  16), ("e8",  "m2",  8,  32), ("e8",  "m4",  8,  64),
    ("e16", "m1",  16,  8), ("e16", "m2",  16, 16), ("e16", "m4",  16, 32),
    ("e32", "m1",  32,  4), ("e32", "m2",  32,  8), ("e32", "m4",  32, 16),
    ("e64", "m1",  64,  2), ("e64", "m2",  64,  4), ("e64", "m4",  64,  8),
    # Fractional LMUL
    ("e8",  "mf2", 8,   8), ("e8",  "mf4", 8,   4), ("e8",  "mf8", 8,   2),
    ("e16", "mf2", 16,  4), ("e16", "mf4", 16,  2),
    ("e32", "mf2", 32,  2),
]

# For widening ops, source SEW must be <= 32 (result fits in 64)
WIDEN_CONFIGS = [
    ("e8",  "m1",  8,  16), ("e8",  "m2",  8,  32),
    ("e16", "m1",  16,  8), ("e16", "m2",  16, 16),
    ("e32", "m1",  32,  4), ("e32", "m2",  32,  8),
]

# Configs with reasonable VLMAX (>= 4) for permutation tests
PERM_CONFIGS = [
    ("e8",  "m1",  8,  16), ("e8",  "m2",  8,  32),
    ("e16", "m1",  16,  8), ("e16", "m2",  16, 16),
    ("e32", "m1",  32,  4), ("e32", "m2",  32,  8),
    ("e64", "m2",  64,  4), ("e64", "m4",  64,  8),
]

# Exact FP64 values (double) as hex bit patterns
EXACT_FP64 = {
    1.0:   0x3FF0000000000000,
    2.0:   0x4000000000000000,
    3.0:   0x4008000000000000,
    4.0:   0x4010000000000000,
    5.0:   0x4014000000000000,
    6.0:   0x4018000000000000,
    7.0:   0x401C000000000000,
    8.0:   0x4020000000000000,
    10.0:  0x4024000000000000,
    12.0:  0x4028000000000000,
    15.0:  0x402E000000000000,
    16.0:  0x4030000000000000,
    0.5:   0x3FE0000000000000,
    0.25:  0x3FD0000000000000,
    -1.0:  0xBFF0000000000000,
    -2.0:  0xC000000000000000,
    -4.0:  0xC010000000000000,
}

# Exact FP32 values (float) as hex bit patterns
EXACT_FP32 = {
    1.0:   0x3F800000,
    2.0:   0x40000000,
    3.0:   0x40400000,
    4.0:   0x40800000,
    5.0:   0x40A00000,
    6.0:   0x40C00000,
    7.0:   0x40E00000,
    8.0:   0x41000000,
    10.0:  0x41200000,
    12.0:  0x41400000,
    16.0:  0x41800000,
    0.5:   0x3F000000,
    0.25:  0x3E800000,
    -1.0:  0xBF800000,
    -2.0:  0xC0000000,
    -4.0:  0xC0800000,
}


def _lmul_numer(lmul_str):
    """Return (numerator, denominator) for LMUL string."""
    if lmul_str.startswith("mf"):
        return (1, int(lmul_str[2:]))
    else:
        return (int(lmul_str[1:]), 1)


def _lmul_nregs(lmul_str):
    """Number of physical registers in a group."""
    n, d = _lmul_numer(lmul_str)
    return max(1, n // d)


def _sew_mask(sew_bits):
    """Unsigned mask for SEW bits."""
    return (1 << sew_bits) - 1


def _sign_extend(val, sew_bits):
    """Sign-extend a SEW-bit value to Python int."""
    mask = _sew_mask(sew_bits)
    val = val & mask
    if val & (1 << (sew_bits - 1)):
        val -= (1 << sew_bits)
    return val


class VecTortureGenerator:
    def __init__(self, seed, length, vec_fp_pct=15):
        self.seed = seed
        self.length = length
        self.vec_fp_pct = vec_fp_pct
        self.testnum = 1
        self.label_id = 0
        self.rng = random.Random(seed)

    # ── Helpers ─────────────────────────────────────────────────────────────

    def next_testnum(self):
        t = self.testnum
        self.testnum += 1
        return t

    def next_label(self, prefix="L"):
        lid = self.label_id
        self.label_id += 1
        return f".{prefix}_{self.seed}_{lid}"

    def rand_scalar_regs(self, n):
        """Pick n distinct scalar registers from pool."""
        return self.rng.sample(SCALAR_REGS, n)

    def rand_vreg(self, lmul_str="m1"):
        """Pick a random LMUL-aligned vector register (not v0, not v1-v7 scratch)."""
        nregs = _lmul_nregs(lmul_str)
        if nregs == 1:
            candidates = list(range(8, 32))
        elif nregs == 2:
            candidates = list(range(8, 32, 2))
        elif nregs == 4:
            candidates = list(range(8, 32, 4))
        elif nregs == 8:
            candidates = [8, 16, 24]
        else:
            candidates = list(range(8, 32))
        return self.rng.choice(candidates)

    def rand_vregs_distinct(self, n, lmul_str="m1"):
        """Pick n distinct non-overlapping LMUL-aligned vector registers."""
        nregs = _lmul_nregs(lmul_str)
        if nregs == 1:
            candidates = list(range(8, 32))
        elif nregs == 2:
            candidates = list(range(8, 32, 2))
        elif nregs == 4:
            candidates = list(range(8, 32, 4))
        elif nregs == 8:
            candidates = [8, 16, 24]
        else:
            candidates = list(range(8, 32))
        return self.rng.sample(candidates, min(n, len(candidates)))

    def rand_config(self, configs=None):
        if configs is None:
            configs = VTYPE_CONFIGS
        return self.rng.choice(configs)

    def rand_sew_value(self, sew_bits):
        return self.rng.randint(0, _sew_mask(sew_bits))

    def rand_small_value(self, sew_bits):
        """Small positive value that fits in SEW, avoids overflow issues."""
        limit = min(127, _sew_mask(sew_bits))
        return self.rng.randint(1, limit)

    def rand_simm5(self):
        """Random 5-bit signed immediate (-16 to 15)."""
        return self.rng.randint(-16, 15)

    # ── Emission helpers ────────────────────────────────────────────────────

    def emit_vsetvli(self, sew_str, lmul_str, ta="ta", ma="ma", avl_reg=None):
        """Emit vsetvli. If avl_reg is None, use zero (VLMAX)."""
        if avl_reg is None:
            return [f"  vsetvli x1, zero, {sew_str}, {lmul_str}, {ta}, {ma}"]
        else:
            return [f"  vsetvli x1, x{avl_reg}, {sew_str}, {lmul_str}, {ta}, {ma}"]

    def emit_check_scalar(self, testnum, got_reg, expected_reg):
        """Emit bne check comparing two scalar registers."""
        return [
            f"  li gp, {testnum}",
            f"  bne x{got_reg}, x{expected_reg}, fail",
        ]

    def emit_truncate_to_sew(self, reg, sew_bits):
        """Truncate/zero-extend a scalar register to SEW bits."""
        if sew_bits >= 64:
            return []
        elif sew_bits == 8:
            return [f"  andi x{reg}, x{reg}, 0xFF"]
        else:
            shift = 64 - sew_bits
            return [
                f"  slli x{reg}, x{reg}, {shift}",
                f"  srli x{reg}, x{reg}, {shift}",
            ]

    def emit_sign_extend_to_sew(self, reg, sew_bits):
        """Sign-extend a SEW-bit value in register to 64 bits."""
        if sew_bits >= 64:
            return []
        shift = 64 - sew_bits
        return [
            f"  slli x{reg}, x{reg}, {shift}",
            f"  srai x{reg}, x{reg}, {shift}",
        ]

    def emit_extract_elem0(self, dst_xreg, vreg):
        """Extract element 0 of vreg to scalar register."""
        return [f"  vmv.x.s x{dst_xreg}, v{vreg}"]

    def emit_extract_elem(self, dst_xreg, vreg, idx, scratch_vreg=2, sew_str=None, lmul_str=None):
        """Extract element idx of vreg to scalar register.

        If sew_str is provided (and lmul_str is None or "m1"), emits a vsetvli
        to e{sew}/m1 first to avoid LMUL alignment issues with the scratch
        register.  If lmul_str is also provided, uses that LMUL instead (needed
        when the element lives in a grouped register beyond the first).
        """
        if idx == 0:
            return self.emit_extract_elem0(dst_xreg, vreg)
        lines = []
        if sew_str is not None:
            lmul = lmul_str if lmul_str else "m1"
            lines.extend(self.emit_vsetvli(sew_str, lmul))
        lines.extend([
            f"  li x{dst_xreg}, {idx}",
            f"  vslidedown.vx v{scratch_vreg}, v{vreg}, x{dst_xreg}",
            f"  vmv.x.s x{dst_xreg}, v{scratch_vreg}",
        ])
        return lines

    # ── Integer ALU block ───────────────────────────────────────────────────

    def gen_int_alu_block(self):
        sew_str, lmul_str, sew, vlmax = self.rand_config()
        mask = _sew_mask(sew)

        # Ops: (asm_vv, asm_vx, asm_vi, python_fn_unsigned)
        vv_ops = [
            ("vadd.vv",  "vadd.vx",  "vadd.vi",  lambda a, b: (a + b) & mask),
            ("vsub.vv",  "vsub.vx",  None,        lambda a, b: (a - b) & mask),
            ("vand.vv",  "vand.vx",  "vand.vi",  lambda a, b: a & b),
            ("vor.vv",   "vor.vx",   "vor.vi",   lambda a, b: a | b),
            ("vxor.vv",  "vxor.vx",  "vxor.vi",  lambda a, b: a ^ b),
            ("vsll.vv",  "vsll.vx",  "vsll.vi",  lambda a, b: (a << (b & (sew - 1))) & mask),
            ("vsrl.vv",  "vsrl.vx",  "vsrl.vi",  lambda a, b: (a & mask) >> (b & (sew - 1))),
            ("vminu.vv", "vminu.vx", None,        lambda a, b: min(a & mask, b & mask)),
            ("vmaxu.vv", "vmaxu.vx", None,        lambda a, b: max(a & mask, b & mask)),
        ]

        op_vv, op_vx, op_vi, fn = self.rng.choice(vv_ops)

        # Pick encoding variant
        variant = self.rng.choice(["vv", "vx", "vi"])
        if variant == "vi" and op_vi is None:
            variant = "vx"

        val_a = self.rng.randint(0, mask)
        val_b = self.rng.randint(0, mask)
        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2, vs1, vd = self.rand_vregs_distinct(3, lmul_str)
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # int_alu: {op_vv} ({sew_str}/{lmul_str})")
        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")

        if variant == "vv":
            lines.append(f"  vmv.v.x v{vs1}, x{r2}")
            lines.append(f"  {op_vv} v{vd}, v{vs2}, v{vs1}")
        elif variant == "vx":
            lines.append(f"  {op_vx} v{vd}, v{vs2}, x{r2}")
        else:
            is_shift = "sll" in op_vi or "srl" in op_vi or "sra" in op_vi
            if is_shift:
                # Shift .vi ops use unsigned immediate 0-31
                uimm5 = self.rng.randint(0, min(31, sew - 1))
                val_b = uimm5
                lines.append(f"  {op_vi} v{vd}, v{vs2}, {uimm5}")
            else:
                simm5 = self.rng.randint(-16, 15)
                val_b = simm5 & mask
                lines.append(f"  {op_vi} v{vd}, v{vs2}, {simm5}")

        expected = fn(val_a, val_b) & mask
        lines.extend(self.emit_extract_elem0(r3, vd))
        lines.append(f"  li x{r4}, {expected}")
        lines.extend(self.emit_truncate_to_sew(r3, sew))
        lines.extend(self.emit_truncate_to_sew(r4, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Integer Multiply block ──────────────────────────────────────────────

    def gen_int_mul_block(self):
        sew_str, lmul_str, sew, vlmax = self.rand_config()
        mask = _sew_mask(sew)

        # Use small values to keep products predictable
        val_a = self.rng.randint(1, min(255, mask))
        val_b = self.rng.randint(1, min(255, mask))

        ops = [
            ("vmul.vv", lambda a, b: (a * b) & mask),
        ]

        # Add vmacc: vd = vs1 * vs2 + vd
        do_macc = self.rng.random() < 0.4

        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vs2, vs1, vd = self.rand_vregs_distinct(3, lmul_str)
        tn = self.next_testnum()

        lines = []
        if do_macc:
            val_acc = self.rng.randint(0, min(255, mask))
            lines.append(f"  # int_mul: vmacc.vv ({sew_str}/{lmul_str})")
            lines.append(f"  li x{r1}, {val_a}")
            lines.append(f"  li x{r2}, {val_b}")
            lines.append(f"  li x{r3}, {val_acc}")
            lines.extend(self.emit_vsetvli(sew_str, lmul_str))
            lines.append(f"  vmv.v.x v{vs1}, x{r1}")
            lines.append(f"  vmv.v.x v{vs2}, x{r2}")
            lines.append(f"  vmv.v.x v{vd}, x{r3}")
            lines.append(f"  vmacc.vv v{vd}, v{vs1}, v{vs2}")
            expected = (val_a * val_b + val_acc) & mask
            lines.extend(self.emit_extract_elem0(r4, vd))
            lines.append(f"  li x{r5}, {expected}")
            lines.extend(self.emit_truncate_to_sew(r4, sew))
            lines.extend(self.emit_truncate_to_sew(r5, sew))
            lines.extend(self.emit_check_scalar(tn, r4, r5))
        else:
            op_asm, fn = self.rng.choice(ops)
            lines.append(f"  # int_mul: {op_asm} ({sew_str}/{lmul_str})")
            lines.append(f"  li x{r1}, {val_a}")
            lines.append(f"  li x{r2}, {val_b}")
            lines.extend(self.emit_vsetvli(sew_str, lmul_str))
            lines.append(f"  vmv.v.x v{vs2}, x{r1}")
            lines.append(f"  vmv.v.x v{vs1}, x{r2}")
            lines.append(f"  {op_asm} v{vd}, v{vs2}, v{vs1}")
            expected = fn(val_a, val_b)
            lines.extend(self.emit_extract_elem0(r3, vd))
            lines.append(f"  li x{r4}, {expected}")
            lines.extend(self.emit_truncate_to_sew(r3, sew))
            lines.extend(self.emit_truncate_to_sew(r4, sew))
            lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Integer Divide block ────────────────────────────────────────────────

    def gen_int_div_block(self):
        sew_str, lmul_str, sew, vlmax = self.rand_config()
        mask = _sew_mask(sew)

        val_a = self.rng.randint(10, min(10000, mask))
        val_b = self.rng.randint(1, min(100, mask))  # never zero

        ops = [
            ("vdivu.vv", lambda a, b: (a // b) & mask if b != 0 else mask),
            ("vremu.vv", lambda a, b: (a % b) & mask if b != 0 else a),
        ]

        op_asm, fn = self.rng.choice(ops)
        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2, vs1, vd = self.rand_vregs_distinct(3, lmul_str)
        tn = self.next_testnum()

        expected = fn(val_a, val_b)

        lines = []
        lines.append(f"  # int_div: {op_asm} ({sew_str}/{lmul_str})")
        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")
        lines.append(f"  {op_asm} v{vd}, v{vs2}, v{vs1}")
        lines.extend(self.emit_extract_elem0(r3, vd))
        lines.append(f"  li x{r4}, {expected}")
        lines.extend(self.emit_truncate_to_sew(r3, sew))
        lines.extend(self.emit_truncate_to_sew(r4, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Unit-stride memory block ────────────────────────────────────────────

    def gen_mem_unit_stride_block(self):
        # Use e64/m1 for simplicity (2 elements at VLEN=128)
        configs = [
            ("e8",  "m1", 8,  16), ("e16", "m1", 16, 8),
            ("e32", "m1", 32,  4), ("e64", "m1", 64,  2),
        ]
        sew_str, lmul_str, sew, vlmax = self.rng.choice(configs)
        elem_bytes = sew // 8

        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vd = self.rand_vreg(lmul_str)
        tn = self.next_testnum()

        val = self.rng.randint(1, _sew_mask(sew))

        store_fn = {8: "sb", 16: "sh", 32: "sw", 64: "sd"}[sew]
        load_fn = {8: "lb", 16: "lh", 32: "lw", 64: "ld"}[sew]
        vle = f"vle{sew}.v"
        vse = f"vse{sew}.v"

        lines = []
        lines.append(f"  # mem_unit_stride: {vle}/{vse} ({sew_str}/{lmul_str})")
        # Store known values to scratch memory
        lines.append(f"  li x{r1}, {val}")
        for i in range(min(vlmax, 4)):
            lines.append(f"  {store_fn} x{r1}, {i * elem_bytes}(x{MEM_BASE_REG})")

        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  {vle} v{vd}, (x{MEM_BASE_REG})")

        # Store to a different offset
        lines.append(f"  addi x{r2}, x{MEM_BASE_REG}, 512")
        lines.append(f"  {vse} v{vd}, (x{r2})")

        # Load back element 0 as scalar and compare
        lines.append(f"  {load_fn} x{r3}, 512(x{MEM_BASE_REG})")
        lines.append(f"  li x{r4}, {val}")
        lines.extend(self.emit_sign_extend_to_sew(r3, sew))
        lines.extend(self.emit_sign_extend_to_sew(r4, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Strided memory block ────────────────────────────────────────────────

    def gen_mem_strided_block(self):
        sew = self.rng.choice([32, 64])
        sew_str = f"e{sew}"
        lmul_str = "m1"
        vlmax = 128 // sew
        elem_bytes = sew // 8
        stride = elem_bytes * self.rng.choice([2, 3, 4])

        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vd = self.rand_vreg(lmul_str)
        tn = self.next_testnum()

        store_fn = {32: "sw", 64: "sd"}[sew]
        load_fn = {32: "lw", 64: "ld"}[sew]

        val = self.rng.randint(1, min(0xFFFF, _sew_mask(sew)))

        lines = []
        lines.append(f"  # mem_strided: vlse{sew}/vsse{sew} stride={stride}")
        lines.append(f"  li x{r1}, {val}")
        # Store values at strided offsets
        for i in range(min(vlmax, 4)):
            lines.append(f"  {store_fn} x{r1}, {i * stride}(x{MEM_BASE_REG})")

        lines.append(f"  li x{r2}, {stride}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vlse{sew}.v v{vd}, (x{MEM_BASE_REG}), x{r2}")

        # Store back unit-stride to verify
        lines.append(f"  addi x{r3}, x{MEM_BASE_REG}, 768")
        lines.append(f"  vse{sew}.v v{vd}, (x{r3})")

        lines.append(f"  {load_fn} x{r4}, 768(x{MEM_BASE_REG})")
        lines.append(f"  li x{r5}, {val}")
        lines.extend(self.emit_sign_extend_to_sew(r4, sew))
        lines.extend(self.emit_sign_extend_to_sew(r5, sew))
        lines.extend(self.emit_check_scalar(tn, r4, r5))
        return lines

    # ── Indexed memory block ────────────────────────────────────────────────

    def gen_mem_indexed_block(self):
        sew = 64
        sew_str = "e64"
        lmul_str = "m1"
        elem_bytes = 8

        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vd, vidx = self.rand_vregs_distinct(2, lmul_str)
        tn = self.next_testnum()

        # Set up data at known offsets
        offset_0 = 0
        offset_1 = 64
        val_0 = self.rng.randint(1, 0xFFFF)
        val_1 = self.rng.randint(1, 0xFFFF)

        lines = []
        lines.append(f"  # mem_indexed: vloxei64")
        lines.append(f"  li x{r1}, {val_0}")
        lines.append(f"  sd x{r1}, {offset_0}(x{MEM_BASE_REG})")
        lines.append(f"  li x{r2}, {val_1}")
        lines.append(f"  sd x{r2}, {offset_1}(x{MEM_BASE_REG})")

        # Store index vector to memory, then load it
        lines.append(f"  li x{r3}, {offset_0}")
        lines.append(f"  sd x{r3}, 1024(x{MEM_BASE_REG})")
        lines.append(f"  li x{r3}, {offset_1}")
        lines.append(f"  sd x{r3}, 1032(x{MEM_BASE_REG})")

        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  addi x{r4}, x{MEM_BASE_REG}, 1024")
        lines.append(f"  vle64.v v{vidx}, (x{r4})")
        lines.append(f"  vloxei64.v v{vd}, (x{MEM_BASE_REG}), v{vidx}")

        # Check element 0
        lines.extend(self.emit_extract_elem0(r5, vd))
        lines.append(f"  li x{r3}, {val_0}")
        lines.extend(self.emit_check_scalar(tn, r5, r3))
        return lines

    # ── Whole-register load/store block ─────────────────────────────────────

    def gen_mem_whole_reg_block(self):
        r1, r2, r3 = self.rand_scalar_regs(3)
        vd = self.rand_vreg("m1")
        tn = self.next_testnum()
        val = self.rng.randint(1, 0xFFFFFFFF)

        lines = []
        lines.append(f"  # mem_whole_reg: vl1re8/vs1r")
        lines.append(f"  li x{r1}, {val}")
        # Fill 16 bytes (VLEN=128 bits)
        lines.append(f"  sd x{r1}, 0(x{MEM_BASE_REG})")
        lines.append(f"  sd x{r1}, 8(x{MEM_BASE_REG})")

        lines.append(f"  vl1re8.v v{vd}, (x{MEM_BASE_REG})")
        lines.append(f"  addi x{r2}, x{MEM_BASE_REG}, 256")
        lines.append(f"  vs1r.v v{vd}, (x{r2})")

        lines.append(f"  ld x{r3}, 256(x{MEM_BASE_REG})")
        lines.append(f"  li x{r2}, {val}")
        lines.extend(self.emit_check_scalar(tn, r3, r2))
        return lines

    # ── Mask load/store block ───────────────────────────────────────────────

    def gen_mem_mask_block(self):
        r1, r2, r3 = self.rand_scalar_regs(3)
        tn = self.next_testnum()
        mask_val = self.rng.randint(1, 255)

        lines = []
        lines.append(f"  # mem_mask: vlm/vsm")
        lines.append(f"  li x{r1}, {mask_val}")
        lines.append(f"  sb x{r1}, 0(x{MEM_BASE_REG})")
        lines.append(f"  li x{r2}, 0")
        lines.append(f"  sb x{r2}, 1(x{MEM_BASE_REG})")

        # Need a vsetvli first so vl is set for vlm/vsm
        lines.extend(self.emit_vsetvli("e8", "m1"))
        lines.append(f"  vlm.v v0, (x{MEM_BASE_REG})")
        lines.append(f"  addi x{r2}, x{MEM_BASE_REG}, 256")
        lines.append(f"  vsm.v v0, (x{r2})")

        lines.append(f"  lb x{r3}, 256(x{MEM_BASE_REG})")
        lines.append(f"  li x{r2}, {mask_val}")
        lines.extend(self.emit_sign_extend_to_sew(r3, 8))
        lines.extend(self.emit_sign_extend_to_sew(r2, 8))
        lines.extend(self.emit_check_scalar(tn, r3, r2))
        return lines

    # ── VL corner cases ─────────────────────────────────────────────────────

    def gen_vl_corner_cases(self):
        case = self.rng.choice(["vl0", "vl1", "vl_lt_vlmax"])
        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vd = self.rand_vreg("m1")
        tn = self.next_testnum()

        lines = []

        if case == "vl0":
            # vl=0: operation should be a no-op
            lines.append(f"  # vl_corner: vl=0 (no-op)")
            sentinel = 0xDEAD
            new_val = 0x1234
            lines.append(f"  li x{r1}, {sentinel}")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.append(f"  vmv.v.x v{vd}, x{r1}")
            # Set vl=0
            lines.append(f"  li x{r2}, 0")
            lines.append(f"  vsetvli x1, x{r2}, e32, m1, ta, ma")
            lines.append(f"  li x{r3}, {new_val}")
            lines.append(f"  vmv.v.x v{vd}, x{r3}")  # should be no-op
            # Restore full vl
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.extend(self.emit_extract_elem0(r4, vd))
            lines.append(f"  li x{r5}, {sentinel}")
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_truncate_to_sew(r5, 32))
            lines.extend(self.emit_check_scalar(tn, r4, r5))

        elif case == "vl1":
            # vl=1: only element 0 modified
            lines.append(f"  # vl_corner: vl=1")
            val = self.rng.randint(1, 0xFFFF)
            lines.append(f"  li x{r1}, {val}")
            lines.append(f"  li x{r2}, 1")
            lines.append(f"  vsetvli x1, x{r2}, e32, m1, ta, ma")
            lines.append(f"  vmv.v.x v{vd}, x{r1}")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.extend(self.emit_extract_elem0(r3, vd))
            lines.append(f"  li x{r4}, {val}")
            lines.extend(self.emit_truncate_to_sew(r3, 32))
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_check_scalar(tn, r3, r4))

        else:
            # vl < VLMAX with tail undisturbed
            lines.append(f"  # vl_corner: vl<VLMAX with tu")
            sentinel = 0xAAAA
            new_val = 0xBBBB
            lines.append(f"  li x{r1}, {sentinel}")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.append(f"  vmv.v.x v{vd}, x{r1}")  # fill all
            lines.append(f"  li x{r2}, {new_val}")
            lines.append(f"  li x{r3}, 2")
            lines.append(f"  vsetvli x1, x{r3}, e32, m1, tu, ma")  # vl=2, tu
            lines.append(f"  vmv.v.x v{vd}, x{r2}")  # only elem 0,1
            # Restore full vl, check element 2 is still sentinel
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.extend(self.emit_extract_elem(r4, vd, 2))
            lines.append(f"  li x{r5}, {sentinel}")
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_truncate_to_sew(r5, 32))
            lines.extend(self.emit_check_scalar(tn, r4, r5))

        return lines

    # ── Masking block ───────────────────────────────────────────────────────

    def gen_masking_block(self):
        r1, r2, r3, r4, r5, r6 = self.rand_scalar_regs(6)
        vs2, vs1, vd = self.rand_vregs_distinct(3, "m1")
        tn = self.next_testnum()

        prefill = 0xAAAA
        val_a = self.rng.randint(1, 0xFFF)
        val_b = self.rng.randint(1, 0xFFF)

        lines = []
        lines.append(f"  # masking: vadd.vv with v0.t (tu, mu)")
        # Pre-fill vd
        lines.append(f"  li x{r1}, {prefill}")
        lines.extend(self.emit_vsetvli("e32", "m1"))
        lines.append(f"  vmv.v.x v{vd}, x{r1}")

        # Set up mask: element 0 active, element 1 inactive (0x55 = 0b01010101)
        lines.append(f"  li x{r2}, 0x55")
        lines.append(f"  sb x{r2}, 0(x{MEM_BASE_REG})")
        lines.append(f"  li x{r2}, 0x55")
        lines.append(f"  sb x{r2}, 1(x{MEM_BASE_REG})")
        lines.append(f"  vlm.v v0, (x{MEM_BASE_REG})")

        # Operands
        lines.append(f"  li x{r3}, {val_a}")
        lines.append(f"  li x{r4}, {val_b}")
        lines.append(f"  vmv.v.x v{vs2}, x{r3}")
        lines.append(f"  vmv.v.x v{vs1}, x{r4}")

        # Masked add with tu/mu
        lines.append(f"  vsetvli x1, zero, e32, m1, tu, mu")
        lines.append(f"  vadd.vv v{vd}, v{vs2}, v{vs1}, v0.t")

        # Restore full vl for extraction
        lines.extend(self.emit_vsetvli("e32", "m1"))

        # Element 0 (active): should be val_a + val_b
        expected_0 = (val_a + val_b) & 0xFFFFFFFF
        lines.extend(self.emit_extract_elem0(r5, vd))
        lines.append(f"  li x{r6}, {expected_0}")
        lines.extend(self.emit_truncate_to_sew(r5, 32))
        lines.extend(self.emit_truncate_to_sew(r6, 32))
        lines.extend(self.emit_check_scalar(tn, r5, r6))

        # Element 1 (inactive, mu): should be original prefill
        tn2 = self.next_testnum()
        lines.extend(self.emit_extract_elem(r5, vd, 1))
        lines.append(f"  li x{r6}, {prefill}")
        lines.extend(self.emit_truncate_to_sew(r5, 32))
        lines.extend(self.emit_truncate_to_sew(r6, 32))
        lines.extend(self.emit_check_scalar(tn2, r5, r6))
        return lines

    # ── Comparison block ────────────────────────────────────────────────────

    def gen_comparison_block(self):
        sew_str, lmul_str, sew, vlmax = self.rand_config()
        mask = _sew_mask(sew)

        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2, vs1 = self.rand_vregs_distinct(2, lmul_str)
        tn = self.next_testnum()

        val_a = self.rng.randint(1, mask // 2)
        val_b = self.rng.randint(val_a + 1, mask) if val_a < mask else val_a

        lines = []
        lines.append(f"  # comparison: vmsltu.vv ({sew_str}/{lmul_str})")
        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")

        # val_a < val_b is always true, so all vl bits should be set
        lines.append(f"  vmsltu.vv v0, v{vs2}, v{vs1}")
        lines.append(f"  vcpop.m x{r3}, v0")
        lines.append(f"  csrr x{r4}, vl")
        lines.extend(self.emit_check_scalar(tn, r3, r4))

        # Also test equality: vmseq with same vector → all bits set
        tn2 = self.next_testnum()
        lines.append(f"  vmseq.vv v0, v{vs2}, v{vs2}")
        lines.append(f"  vcpop.m x{r3}, v0")
        lines.extend(self.emit_check_scalar(tn2, r3, r4))
        return lines

    # ── Reduction block ─────────────────────────────────────────────────────

    def gen_reduction_block(self):
        # Use configs with small enough vlmax that sum doesn't overflow
        configs = [
            ("e32", "m1", 32, 4), ("e32", "m2", 32, 8),
            ("e64", "m1", 64, 2), ("e64", "m2", 64, 4),
        ]
        sew_str, lmul_str, sew, vlmax = self.rng.choice(configs)
        mask = _sew_mask(sew)

        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vs2 = self.rand_vreg(lmul_str)
        vs1 = self.rand_vreg("m1")  # accumulator is always m1
        while vs1 == vs2 or (vs1 >= vs2 and vs1 < vs2 + _lmul_nregs(lmul_str)):
            vs1 = self.rand_vreg("m1")
        vd = self.rand_vreg("m1")
        while vd == vs2 or vd == vs1 or (vd >= vs2 and vd < vs2 + _lmul_nregs(lmul_str)):
            vd = self.rand_vreg("m1")
        tn = self.next_testnum()

        elem_val = self.rng.randint(1, min(100, mask // max(vlmax, 1)))
        acc_val = self.rng.randint(0, min(1000, mask // 2))

        lines = []
        lines.append(f"  # reduction: vredsum.vs ({sew_str}/{lmul_str})")
        lines.append(f"  li x{r1}, {elem_val}")
        lines.append(f"  li x{r2}, {acc_val}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        # For accumulator, set just element 0
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")

        lines.append(f"  vredsum.vs v{vd}, v{vs2}, v{vs1}")
        lines.extend(self.emit_extract_elem0(r3, vd))

        # Expected: elem_val * vl + acc_val
        lines.append(f"  csrr x{r4}, vl")
        lines.append(f"  mul x{r5}, x{r1}, x{r4}")
        lines.append(f"  add x{r5}, x{r5}, x{r2}")
        lines.extend(self.emit_truncate_to_sew(r3, sew))
        lines.extend(self.emit_truncate_to_sew(r5, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r5))
        return lines

    # ── Permutation block ───────────────────────────────────────────────────

    def gen_permute_block(self):
        kind = self.rng.choice(["slidedown", "slide1down", "rgather"])
        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        tn = self.next_testnum()

        lines = []
        if kind == "slidedown":
            # Build known vector via memory, slidedown by 1, check element 0
            lines.append(f"  # permute: vslidedown")
            vals = [self.rng.randint(1, 0xFFFF) for _ in range(4)]
            for i, v in enumerate(vals):
                lines.append(f"  li x{r1}, {v}")
                lines.append(f"  sw x{r1}, {i * 4}(x{MEM_BASE_REG})")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            vd, vs2 = self.rand_vregs_distinct(2, "m1")
            lines.append(f"  vle32.v v{vs2}, (x{MEM_BASE_REG})")
            lines.append(f"  li x{r2}, 1")
            lines.append(f"  vslidedown.vx v{vd}, v{vs2}, x{r2}")
            lines.extend(self.emit_extract_elem0(r3, vd))
            lines.append(f"  li x{r4}, {vals[1]}")
            lines.extend(self.emit_truncate_to_sew(r3, 32))
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_check_scalar(tn, r3, r4))

        elif kind == "slide1down":
            # vslide1down.vx inserts scalar at the end
            lines.append(f"  # permute: vslide1down")
            val = self.rng.randint(1, 0xFFFF)
            insert_val = self.rng.randint(1, 0xFFFF)
            lines.append(f"  li x{r1}, {val}")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            vs2 = self.rand_vreg("m1")
            vd = self.rand_vreg("m1")
            while vd == vs2:
                vd = self.rand_vreg("m1")
            lines.append(f"  vmv.v.x v{vs2}, x{r1}")
            lines.append(f"  li x{r2}, {insert_val}")
            lines.append(f"  vslide1down.vx v{vd}, v{vs2}, x{r2}")
            # Element 0 should be the old element 1 = val
            lines.extend(self.emit_extract_elem0(r3, vd))
            lines.append(f"  li x{r4}, {val}")
            lines.extend(self.emit_truncate_to_sew(r3, 32))
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_check_scalar(tn, r3, r4))

        else:  # rgather
            # vrgather: v8=[10,20,30,40], v12=[2,0,3,1] → v16=[30,10,40,20]
            lines.append(f"  # permute: vrgather")
            data = [10, 20, 30, 40]
            indices = [2, 0, 3, 1]
            for i, v in enumerate(data):
                lines.append(f"  li x{r1}, {v}")
                lines.append(f"  sw x{r1}, {i * 4}(x{MEM_BASE_REG})")
            for i, v in enumerate(indices):
                lines.append(f"  li x{r1}, {v}")
                lines.append(f"  sw x{r1}, {(i + 4) * 4}(x{MEM_BASE_REG})")

            lines.extend(self.emit_vsetvli("e32", "m1"))
            vs2, vidx, vd = self.rand_vregs_distinct(3, "m1")
            lines.append(f"  vle32.v v{vs2}, (x{MEM_BASE_REG})")
            lines.append(f"  addi x{r2}, x{MEM_BASE_REG}, 16")
            lines.append(f"  vle32.v v{vidx}, (x{r2})")
            lines.append(f"  vrgather.vv v{vd}, v{vs2}, v{vidx}")
            # Element 0 should be data[indices[0]] = data[2] = 30
            lines.extend(self.emit_extract_elem0(r3, vd))
            lines.append(f"  li x{r4}, {data[indices[0]]}")
            lines.extend(self.emit_truncate_to_sew(r3, 32))
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_check_scalar(tn, r3, r4))

        return lines

    # ── Widening block ──────────────────────────────────────────────────────

    def gen_widening_block(self):
        sew_str, lmul_str, sew, vlmax = self.rng.choice(WIDEN_CONFIGS)
        mask = _sew_mask(sew)
        dst_sew = sew * 2
        dst_sew_str = f"e{dst_sew}"
        # Destination LMUL is 2× source
        dst_nregs = _lmul_nregs(lmul_str) * 2
        dst_lmul_str = f"m{dst_nregs}"

        val_a = self.rng.randint(1, min(1000, mask))
        val_b = self.rng.randint(1, min(1000, mask))

        r1, r2, r3, r4 = self.rand_scalar_regs(4)

        # Source regs are LMUL-aligned, dest is 2×LMUL-aligned
        # Need to ensure no overlap
        vs2 = self.rand_vreg(lmul_str)
        vs1 = self.rand_vreg(lmul_str)
        vd = self.rand_vreg(dst_lmul_str)
        # Ensure no overlap between vd group and vs2/vs1
        attempts = 0
        while attempts < 20:
            vd_range = set(range(vd, vd + dst_nregs))
            vs2_range = set(range(vs2, vs2 + _lmul_nregs(lmul_str)))
            vs1_range = set(range(vs1, vs1 + _lmul_nregs(lmul_str)))
            if not (vd_range & vs2_range) and not (vd_range & vs1_range) and not (vs2_range & vs1_range):
                break
            vs2 = self.rand_vreg(lmul_str)
            vs1 = self.rand_vreg(lmul_str)
            vd = self.rand_vreg(dst_lmul_str)
            attempts += 1

        tn = self.next_testnum()

        op = self.rng.choice(["vwadd", "vwsub", "vwmulu"])

        lines = []
        lines.append(f"  # widening: {op} ({sew_str}/{lmul_str} -> {dst_sew_str}/{dst_lmul_str})")
        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")

        if op == "vwadd":
            lines.append(f"  vwaddu.vv v{vd}, v{vs2}, v{vs1}")
            expected = val_a + val_b
        elif op == "vwsub":
            # Ensure a >= b for unsigned
            if val_a < val_b:
                val_a, val_b = val_b, val_a
                lines[1] = f"  li x{r1}, {val_a}"
                lines[2] = f"  li x{r2}, {val_b}"
            lines.append(f"  vwsubu.vv v{vd}, v{vs2}, v{vs1}")
            expected = val_a - val_b
        else:
            lines.append(f"  vwmulu.vv v{vd}, v{vs2}, v{vs1}")
            expected = val_a * val_b

        # Switch to result SEW to extract
        lines.extend(self.emit_vsetvli(dst_sew_str, dst_lmul_str))
        lines.extend(self.emit_extract_elem0(r3, vd))
        lines.append(f"  li x{r4}, {expected}")
        lines.extend(self.emit_truncate_to_sew(r3, dst_sew))
        lines.extend(self.emit_truncate_to_sew(r4, dst_sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Narrowing block ─────────────────────────────────────────────────────

    def gen_narrowing_block(self):
        # Source is wide, result is narrow
        # Use e32 source → e16 result, or e64 → e32
        narrow = self.rng.choice([(32, 16, "m2", "m1"), (64, 32, "m2", "m1")])
        src_sew, dst_sew, src_lmul, dst_lmul = narrow
        src_mask = _sew_mask(src_sew)
        dst_mask = _sew_mask(dst_sew)

        val = self.rng.randint(0, dst_mask)  # fits in narrow result

        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2 = self.rand_vreg(src_lmul)
        vd = self.rand_vreg(dst_lmul)
        # Ensure no overlap
        attempts = 0
        while attempts < 20:
            vs2_range = set(range(vs2, vs2 + _lmul_nregs(src_lmul)))
            vd_range = set(range(vd, vd + _lmul_nregs(dst_lmul)))
            if not (vs2_range & vd_range):
                break
            vs2 = self.rand_vreg(src_lmul)
            vd = self.rand_vreg(dst_lmul)
            attempts += 1
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # narrowing: vnsrl e{src_sew} -> e{dst_sew}")
        lines.append(f"  li x{r1}, {val}")
        lines.extend(self.emit_vsetvli(f"e{src_sew}", src_lmul))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")

        # vnsrl with shift=0: just take lower bits
        lines.extend(self.emit_vsetvli(f"e{dst_sew}", dst_lmul))
        lines.append(f"  li x{r2}, 0")
        lines.append(f"  vnsrl.wx v{vd}, v{vs2}, x{r2}")

        lines.extend(self.emit_extract_elem0(r3, vd))
        lines.append(f"  li x{r4}, {val}")
        lines.extend(self.emit_truncate_to_sew(r3, dst_sew))
        lines.extend(self.emit_truncate_to_sew(r4, dst_sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Mask operations block ───────────────────────────────────────────────

    def gen_mask_ops_block(self):
        kind = self.rng.choice(["vcpop", "vfirst", "vid", "viota"])
        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        tn = self.next_testnum()

        lines = []
        if kind == "vcpop":
            mask_val = self.rng.randint(1, 15)  # 4-bit pattern
            popcount = bin(mask_val).count("1")
            lines.append(f"  # mask_ops: vcpop.m")
            lines.append(f"  li x{r1}, {mask_val}")
            lines.append(f"  sb x{r1}, 0(x{MEM_BASE_REG})")
            lines.append(f"  li x{r1}, 0")
            lines.append(f"  sb x{r1}, 1(x{MEM_BASE_REG})")
            lines.extend(self.emit_vsetvli("e32", "m1"))  # vl=4
            lines.append(f"  vlm.v v8, (x{MEM_BASE_REG})")
            lines.append(f"  vcpop.m x{r2}, v8")
            lines.append(f"  li x{r3}, {popcount}")
            lines.extend(self.emit_check_scalar(tn, r2, r3))

        elif kind == "vfirst":
            # Pick a random bit position for the first set bit (0-3)
            first_pos = self.rng.randint(0, 3)
            mask_val = 1 << first_pos
            # Add random higher bits
            mask_val |= self.rng.randint(0, 15) << (first_pos + 1)
            mask_val &= 0xF  # keep to 4 bits
            if mask_val == 0:
                mask_val = 1 << first_pos
            # Recompute actual first position
            for i in range(4):
                if mask_val & (1 << i):
                    first_pos = i
                    break
            lines.append(f"  # mask_ops: vfirst.m")
            lines.append(f"  li x{r1}, {mask_val}")
            lines.append(f"  sb x{r1}, 0(x{MEM_BASE_REG})")
            lines.append(f"  li x{r1}, 0")
            lines.append(f"  sb x{r1}, 1(x{MEM_BASE_REG})")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.append(f"  vlm.v v8, (x{MEM_BASE_REG})")
            lines.append(f"  vfirst.m x{r2}, v8")
            lines.append(f"  li x{r3}, {first_pos}")
            lines.extend(self.emit_check_scalar(tn, r2, r3))

        elif kind == "vid":
            # vid.v: v[i] = i
            lines.append(f"  # mask_ops: vid.v")
            vd = self.rand_vreg("m1")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.append(f"  vid.v v{vd}")
            # Check element 0 = 0
            lines.extend(self.emit_extract_elem0(r2, vd))
            lines.append(f"  li x{r3}, 0")
            lines.extend(self.emit_truncate_to_sew(r2, 32))
            lines.extend(self.emit_check_scalar(tn, r2, r3))
            # Check element 2 = 2
            tn2 = self.next_testnum()
            lines.extend(self.emit_extract_elem(r4, vd, 2))
            lines.append(f"  li x{r5}, 2")
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_check_scalar(tn2, r4, r5))

        else:  # viota
            # mask = 0b1011 → viota = [0, 1, 1, 2]
            lines.append(f"  # mask_ops: viota.m")
            mask_val = 0b1011
            lines.append(f"  li x{r1}, {mask_val}")
            lines.append(f"  sb x{r1}, 0(x{MEM_BASE_REG})")
            lines.append(f"  li x{r1}, 0")
            lines.append(f"  sb x{r1}, 1(x{MEM_BASE_REG})")
            lines.extend(self.emit_vsetvli("e32", "m1"))
            lines.append(f"  vlm.v v0, (x{MEM_BASE_REG})")
            vd = self.rand_vreg("m1")
            lines.append(f"  viota.m v{vd}, v0")
            # Element 0: 0 bits before position 0 → 0
            lines.extend(self.emit_extract_elem0(r2, vd))
            lines.append(f"  li x{r3}, 0")
            lines.extend(self.emit_truncate_to_sew(r2, 32))
            lines.extend(self.emit_check_scalar(tn, r2, r3))
            # Element 3: bits set at positions 0,1 (mask=0b1011) → prefix pop of 0b101 = 2
            tn2 = self.next_testnum()
            lines.extend(self.emit_extract_elem(r4, vd, 3))
            lines.append(f"  li x{r5}, 2")
            lines.extend(self.emit_truncate_to_sew(r4, 32))
            lines.extend(self.emit_check_scalar(tn2, r4, r5))

        return lines

    # ── vsetvl chain block ──────────────────────────────────────────────────

    def gen_vsetvl_chain_block(self):
        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # vsetvl_chain: vsetvli result used by scalar")
        # AVL = 4, e32/m1 → VLMAX=4 (at VLEN=128), so vl = min(4,4) = 4
        lines.append(f"  li x{r1}, 4")
        lines.append(f"  vsetvli x{r2}, x{r1}, e32, m1, ta, ma")
        lines.append(f"  addi x{r3}, x{r2}, 1")  # should be 5
        lines.append(f"  li x{r4}, 5")
        lines.extend(self.emit_check_scalar(tn, r3, r4))

        # Test AVL > VLMAX
        tn2 = self.next_testnum()
        lines.append(f"  li x{r1}, 9999")
        lines.append(f"  vsetvli x{r2}, x{r1}, e32, m1, ta, ma")
        # Result should be VLMAX; use csrr vlenb to compute expected
        lines.append(f"  csrr x{r3}, vlenb")  # VLEN/8 in bytes
        lines.append(f"  srli x{r3}, x{r3}, 2")  # / 4 for e32 → VLMAX for m1
        lines.extend(self.emit_check_scalar(tn2, r2, r3))
        return lines

    # ── LMUL grouping block ─────────────────────────────────────────────────

    def gen_lmul_grouping_block(self):
        # m2: each operand spans 2 registers
        lmul_str = self.rng.choice(["m2", "m4"])
        nregs = _lmul_nregs(lmul_str)
        sew = 32
        sew_str = "e32"
        vlmax = (128 * nregs) // sew  # for VLEN=128

        val_a = self.rng.randint(1, 100)
        val_b = self.rng.randint(1, 100)
        expected = (val_a + val_b) & 0xFFFFFFFF

        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vs2, vs1, vd = self.rand_vregs_distinct(3, lmul_str)
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # lmul_group: vadd.vv ({sew_str}/{lmul_str})")
        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")
        lines.append(f"  vadd.vv v{vd}, v{vs2}, v{vs1}")

        # Check element 0
        lines.extend(self.emit_extract_elem0(r3, vd))
        lines.append(f"  li x{r4}, {expected}")
        lines.extend(self.emit_truncate_to_sew(r3, sew))
        lines.extend(self.emit_truncate_to_sew(r4, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))

        # Check an element in the second register of the group
        # Element nregs*4/2 = in the middle of the group
        elem_idx = (128 // sew)  # elements per register = 4, check element 4 (in 2nd vreg)
        if elem_idx < vlmax:
            tn2 = self.next_testnum()
            # Must keep original LMUL so vslidedown can see the full group.
            # Use an LMUL-aligned scratch register (v2 for m2, v4 for m4).
            scratch = nregs if nregs <= 4 else 8  # m2→v2, m4→v4
            lines.extend(self.emit_extract_elem(r3, vd, elem_idx,
                                                scratch_vreg=scratch,
                                                sew_str=sew_str,
                                                lmul_str=lmul_str))
            lines.append(f"  li x{r5}, {expected}")
            lines.extend(self.emit_truncate_to_sew(r3, sew))
            lines.extend(self.emit_truncate_to_sew(r5, sew))
            lines.extend(self.emit_check_scalar(tn2, r3, r5))

        return lines

    # ── Fractional LMUL block ───────────────────────────────────────────────

    def gen_fractional_lmul_block(self):
        frac_configs = [
            ("e8",  "mf2", 8,   8),
            ("e8",  "mf4", 8,   4),
            ("e8",  "mf8", 8,   2),
            ("e16", "mf2", 16,  4),
            ("e16", "mf4", 16,  2),
            ("e32", "mf2", 32,  2),
        ]
        sew_str, lmul_str, sew, vlmax = self.rng.choice(frac_configs)
        mask = _sew_mask(sew)

        val_a = self.rng.randint(1, min(100, mask // 2))
        val_b = self.rng.randint(1, min(100, mask // 2))
        expected = (val_a + val_b) & mask

        r1, r2, r3, r4, r5, r6 = self.rand_scalar_regs(6)
        vs2, vs1, vd = self.rand_vregs_distinct(3, "m1")  # fractional uses m1 alignment
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # fractional_lmul: vadd ({sew_str}/{lmul_str})")
        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")
        lines.append(f"  vadd.vv v{vd}, v{vs2}, v{vs1}")
        lines.extend(self.emit_extract_elem0(r3, vd))
        lines.append(f"  li x{r4}, {expected}")
        lines.extend(self.emit_truncate_to_sew(r3, sew))
        lines.extend(self.emit_truncate_to_sew(r4, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))

        # Verify vl was set correctly
        tn2 = self.next_testnum()
        lines.extend(self.emit_vsetvli(sew_str, lmul_str))
        lines.append(f"  csrr x{r5}, vl")
        lines.append(f"  li x{r6}, {vlmax}")
        lines.extend(self.emit_check_scalar(tn2, r5, r6))
        return lines

    # ── Extension block ─────────────────────────────────────────────────────

    def gen_extension_block(self):
        # vzext.vf2: zero-extend e16 → e32
        r1, r2, r3 = self.rand_scalar_regs(3)
        vs2 = self.rand_vreg("m1")
        vd = self.rand_vreg("m1")
        while vd == vs2:
            vd = self.rand_vreg("m1")
        tn = self.next_testnum()

        val = 0xFFFF  # max u16
        lines = []
        lines.append(f"  # extension: vzext.vf2 e16 -> e32")
        lines.append(f"  li x{r1}, {val}")
        lines.extend(self.emit_vsetvli("e16", "m1"))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.extend(self.emit_vsetvli("e32", "m1"))
        lines.append(f"  vzext.vf2 v{vd}, v{vs2}")
        lines.extend(self.emit_extract_elem0(r2, vd))
        lines.append(f"  li x{r3}, {val}")
        lines.extend(self.emit_truncate_to_sew(r2, 32))
        lines.extend(self.emit_truncate_to_sew(r3, 32))
        lines.extend(self.emit_check_scalar(tn, r2, r3))
        return lines

    # ── Saturating block ────────────────────────────────────────────────────

    def gen_saturating_block(self):
        sew = self.rng.choice([8, 16, 32])
        sew_str = f"e{sew}"
        mask = _sew_mask(sew)

        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2, vs1, vd = self.rand_vregs_distinct(3, "m1")
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # saturating: vsaddu.vv ({sew_str}/m1)")
        lines.append(f"  li x{r1}, {mask}")  # max unsigned value
        lines.append(f"  li x{r2}, 1")
        lines.extend(self.emit_vsetvli(sew_str, "m1"))
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")
        lines.append(f"  vsaddu.vv v{vd}, v{vs2}, v{vs1}")
        lines.extend(self.emit_extract_elem0(r3, vd))
        # Saturates to max
        lines.append(f"  li x{r4}, {mask}")
        lines.extend(self.emit_truncate_to_sew(r3, sew))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── Add/subtract with carry block ───────────────────────────────────────

    def gen_adc_sbc_block(self):
        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vs2, vs1, vd = self.rand_vregs_distinct(3, "m1")
        tn = self.next_testnum()

        val_a = self.rng.randint(1, 1000)
        val_b = self.rng.randint(1, 1000)

        lines = []
        lines.append(f"  # adc: vadc.vvm")
        # Set up v0 mask with bit 0 = 1 (carry-in for element 0)
        lines.append(f"  li x{r1}, 0x01")
        lines.append(f"  sb x{r1}, 0(x{MEM_BASE_REG})")
        lines.append(f"  li x{r1}, 0")
        lines.append(f"  sb x{r1}, 1(x{MEM_BASE_REG})")

        lines.extend(self.emit_vsetvli("e32", "m1"))
        lines.append(f"  vlm.v v0, (x{MEM_BASE_REG})")

        lines.append(f"  li x{r1}, {val_a}")
        lines.append(f"  li x{r2}, {val_b}")
        lines.append(f"  vmv.v.x v{vs2}, x{r1}")
        lines.append(f"  vmv.v.x v{vs1}, x{r2}")
        lines.append(f"  vadc.vvm v{vd}, v{vs2}, v{vs1}, v0")
        lines.extend(self.emit_extract_elem0(r3, vd))
        expected = (val_a + val_b + 1) & 0xFFFFFFFF  # +1 for carry bit
        lines.append(f"  li x{r4}, {expected}")
        lines.extend(self.emit_truncate_to_sew(r3, 32))
        lines.extend(self.emit_truncate_to_sew(r4, 32))
        lines.extend(self.emit_check_scalar(tn, r3, r4))
        return lines

    # ── FP ALU block ────────────────────────────────────────────────────────

    def gen_fp_alu_block(self):
        use_e32 = self.rng.random() < 0.4
        if use_e32:
            sew_str, sew = "e32", 32
            fp_table = EXACT_FP32
            fmv_in = "fmv.w.x"
            feq_fn = "feq.s"
            vfmv_in = "vfmv.v.f"
            vfmv_out = "vfmv.f.s"
        else:
            sew_str, sew = "e64", 64
            fp_table = EXACT_FP64
            fmv_in = "fmv.d.x"
            feq_fn = "feq.d"
            vfmv_in = "vfmv.v.f"
            vfmv_out = "vfmv.f.s"

        ops = [
            ("vfadd.vv", lambda a, b: a + b),
            ("vfsub.vv", lambda a, b: a - b),
            ("vfmul.vv", lambda a, b: a * b),
        ]
        op_asm, fn = self.rng.choice(ops)

        # Pick operands that produce exact results
        fp_vals = list(fp_table.keys())
        for _ in range(100):
            a_val = self.rng.choice(fp_vals)
            b_val = self.rng.choice(fp_vals)
            result = fn(a_val, b_val)
            if result in fp_table:
                break
        else:
            # Fallback to safe values
            a_val, b_val = 4.0, 8.0
            result = fn(a_val, b_val)
            if result not in fp_table:
                a_val, b_val = 2.0, 3.0
                result = fn(a_val, b_val)

        if result not in fp_table:
            # Use add with safe values
            op_asm = "vfadd.vv"
            a_val, b_val = 4.0, 8.0
            result = 12.0

        a_hex = fp_table[a_val]
        b_hex = fp_table[b_val]
        r_hex = fp_table[result]

        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2, vs1, vd = self.rand_vregs_distinct(3, "m1")
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # fp_alu: {op_asm} ({sew_str}) {a_val} op {b_val} = {result}")
        lines.append(f"  li x{r1}, 0x{a_hex:X}")
        lines.append(f"  li x{r2}, 0x{b_hex:X}")
        lines.append(f"  {fmv_in} f1, x{r1}")
        lines.append(f"  {fmv_in} f2, x{r2}")

        lines.extend(self.emit_vsetvli(sew_str, "m1"))
        lines.append(f"  {vfmv_in} v{vs2}, f1")
        lines.append(f"  {vfmv_in} v{vs1}, f2")
        lines.append(f"  {op_asm} v{vd}, v{vs2}, v{vs1}")
        lines.append(f"  {vfmv_out} f3, v{vd}")

        lines.append(f"  li x{r3}, 0x{r_hex:X}")
        lines.append(f"  {fmv_in} f4, x{r3}")
        lines.append(f"  {feq_fn} x{r4}, f3, f4")
        lines.append(f"  li gp, {tn}")
        lines.append(f"  beqz x{r4}, fail")
        return lines

    # ── FP FMA block ────────────────────────────────────────────────────────

    def gen_fp_fma_block(self):
        # vfmacc.vv: vd = vs1 * vs2 + vd
        # Use: 2.0 * 3.0 + 4.0 = 10.0
        fp_table = EXACT_FP64
        sew_str = "e64"

        # Find a valid triple (a*b + c) = result, all exact
        triples = [
            (2.0, 3.0, 4.0, 10.0),
            (2.0, 4.0, 2.0, 10.0),
            (3.0, 4.0, 3.0, 15.0),
            (2.0, 2.0, 4.0, 8.0),
            (4.0, 2.0, -1.0, 7.0),
        ]
        a, b, c, result = self.rng.choice(triples)

        r1, r2, r3, r4, r5 = self.rand_scalar_regs(5)
        vs1, vs2, vd = self.rand_vregs_distinct(3, "m1")
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # fp_fma: vfmacc.vv {a}*{b}+{c}={result}")
        lines.append(f"  li x{r1}, 0x{fp_table[a]:X}")
        lines.append(f"  li x{r2}, 0x{fp_table[b]:X}")
        lines.append(f"  li x{r3}, 0x{fp_table[c]:X}")
        lines.append(f"  fmv.d.x f1, x{r1}")
        lines.append(f"  fmv.d.x f2, x{r2}")
        lines.append(f"  fmv.d.x f3, x{r3}")

        lines.extend(self.emit_vsetvli(sew_str, "m1"))
        lines.append(f"  vfmv.v.f v{vs1}, f1")
        lines.append(f"  vfmv.v.f v{vs2}, f2")
        lines.append(f"  vfmv.v.f v{vd}, f3")  # accumulator
        lines.append(f"  vfmacc.vv v{vd}, v{vs1}, v{vs2}")

        lines.append(f"  vfmv.f.s f4, v{vd}")
        lines.append(f"  li x{r4}, 0x{fp_table[result]:X}")
        lines.append(f"  fmv.d.x f5, x{r4}")
        lines.append(f"  feq.d x{r5}, f4, f5")
        lines.append(f"  li gp, {tn}")
        lines.append(f"  beqz x{r5}, fail")
        return lines

    # ── Conversion block ────────────────────────────────────────────────────

    def gen_conversion_block(self):
        # vfcvt.x.f.v: float → int (round to nearest)
        # Use exact integer values as floats
        fp_table = EXACT_FP32
        int_val = self.rng.choice([1, 2, 3, 4, 5, 6, 7, 8])
        float_val = float(int_val)
        fp_hex = fp_table[float_val]

        r1, r2, r3, r4 = self.rand_scalar_regs(4)
        vs2, vd = self.rand_vregs_distinct(2, "m1")
        tn = self.next_testnum()

        lines = []
        lines.append(f"  # conversion: vfcvt.x.f.v (e32) {float_val} -> {int_val}")
        lines.append(f"  li x{r1}, 0x{fp_hex:X}")
        lines.append(f"  fmv.w.x f1, x{r1}")
        lines.extend(self.emit_vsetvli("e32", "m1"))
        lines.append(f"  vfmv.v.f v{vs2}, f1")
        lines.append(f"  vfcvt.x.f.v v{vd}, v{vs2}")
        lines.extend(self.emit_extract_elem0(r2, vd))
        lines.append(f"  li x{r3}, {int_val}")
        lines.extend(self.emit_truncate_to_sew(r2, 32))
        lines.extend(self.emit_truncate_to_sew(r3, 32))
        lines.extend(self.emit_check_scalar(tn, r2, r3))
        return lines

    # ── Top-level generate ──────────────────────────────────────────────────

    def generate(self):
        lines = []
        lines.append(f"# Vector torture test -- seed {self.seed}, length {self.length}")
        lines.append('#include "riscv_test.h"')
        lines.append('#include "test_macros.h"')
        lines.append("")
        lines.append("RVTEST_RV64UV")
        lines.append("RVTEST_CODE_BEGIN")
        lines.append("")

        # Initialize scalar registers with known values
        lines.append("  # Initialize scalar registers")
        for r in SCALAR_REGS:
            val = self.rng.randint(1, (1 << 63) - 1)
            lines.append(f"  li x{r}, 0x{val:016x}")
        lines.append("")

        # Set up memory base
        lines.append("  la x4, scratch_data")
        lines.append("")

        # Initialize scratch memory (2KB, max offset 2040 for sd imm12)
        lines.append("  # Initialize scratch memory")
        lines.append("  li x5, 0xCAFEBABEDEADBEEF")
        for off in range(0, 2048, 8):
            lines.append(f"  sd x5, {off}(x4)")
        lines.append("")

        # Re-init x5
        rng_backup = self.rng.getstate()
        self.rng.setstate(rng_backup)

        # Block generators with weights
        block_generators = [
            (30, self.gen_int_alu_block),
            (15, self.gen_mem_unit_stride_block),
            (10, self.gen_vl_corner_cases),
            (10, self.gen_masking_block),
            (8,  self.gen_int_mul_block),
            (5,  self.gen_int_div_block),
            (8,  self.gen_reduction_block),
            (8,  self.gen_lmul_grouping_block),
            (6,  self.gen_mem_strided_block),
            (6,  self.gen_mem_indexed_block),
            (8,  self.gen_permute_block),
            (5,  self.gen_widening_block),
            (4,  self.gen_narrowing_block),
            (5,  self.gen_comparison_block),
            (5,  self.gen_mask_ops_block),
            (4,  self.gen_vsetvl_chain_block),
            (4,  self.gen_fractional_lmul_block),
            (3,  self.gen_extension_block),
            (3,  self.gen_saturating_block),
            (3,  self.gen_adc_sbc_block),
            (3,  self.gen_mem_whole_reg_block),
            (3,  self.gen_mem_mask_block),
        ]

        if self.vec_fp_pct > 0:
            block_generators += [
                (5, self.gen_fp_alu_block),
                (4, self.gen_fp_fma_block),
                (3, self.gen_conversion_block),
            ]

        weights, generators = zip(*block_generators)

        for i in range(self.length):
            gen_fn = self.rng.choices(generators, weights=weights, k=1)[0]
            block_lines = gen_fn()
            lines.append(f"  # --- Block {i} ({gen_fn.__name__}) ---")
            lines.extend(block_lines)
            lines.append("")

        # Pass
        lines.append("  RVTEST_PASS")
        lines.append("")
        lines.append("RVTEST_CODE_END")
        lines.append("")

        # Fail handler
        lines.append("  .align 2")
        lines.append("fail:")
        lines.append("  RVTEST_FAIL")
        lines.append("")

        # Data section
        lines.append("  .data")
        lines.append("RVTEST_DATA_BEGIN")
        lines.append("")
        lines.append("  .align 4")
        lines.append("scratch_data:")
        lines.append("  .fill 2048, 1, 0")
        lines.append("")
        lines.append("RVTEST_DATA_END")

        return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description="Generate RVV vector torture tests")
    ap.add_argument("--seed", type=int, default=0, help="Starting seed")
    ap.add_argument("--count", type=int, default=100, help="Number of tests to generate")
    ap.add_argument("--length", type=int, default=50, help="Number of test blocks per test")
    ap.add_argument(
        "--outdir",
        default=os.path.join(
            os.path.dirname(os.path.abspath(__file__)), "generated_vec"
        ),
        help="Output directory",
    )
    ap.add_argument("--fp-pct", type=int, default=15, help="FP test percentage (0 to disable)")
    args = ap.parse_args()

    os.makedirs(args.outdir, exist_ok=True)

    for i in range(args.count):
        seed = args.seed + i
        gen = VecTortureGenerator(
            seed=seed,
            length=args.length,
            vec_fp_pct=args.fp_pct,
        )
        code = gen.generate()
        path = os.path.join(args.outdir, f"vec_torture_{seed:06d}.S")
        with open(path, "w") as f:
            f.write(code)

    print(f"Generated {args.count} vector torture tests in {args.outdir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
