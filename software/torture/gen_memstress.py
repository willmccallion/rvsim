#!/usr/bin/env python3
"""Generate aggressive memory stress tests targeting subtle OoO pipeline bugs.

These tests go beyond basic store-load verification to stress:
  1. Store buffer capacity boundaries (exact SB size flooding)
  2. Back-to-back stores to SAME address (last-store-wins)
  3. Rapid store-then-load with maximum ROB pressure (many in-flight ops)
  4. Store forwarding across different widths in rapid succession
  5. Memory-dependent chains (pointer chase patterns)
  6. WAW hazards (multiple stores to same addr, verify last one wins)
  7. RAW through store buffer with maximum intervening ops
  8. Stores interleaved with AMOs to same address

The key insight: the Linux crash shows two loads from the SAME address
returning DIFFERENT values. This could be caused by:
  - Store buffer forwarding returning stale data
  - A store being dropped during ROB squash
  - A load bypassing an older store incorrectly
  - Memory ordering violation not being detected

Usage:
    python gen_memstress.py --seed 0 --count 500 --length 500
"""

import argparse
import os
import random
import sys

REGS = list(range(5, 31))
MEM_BASE_REG = 4
CHECK_REG = 31
MAX_OFF = 2040


def rand_reg():
    return random.choice(REGS)


def rand_reg_not(*exclude):
    pool = [r for r in REGS if r not in exclude]
    return random.choice(pool)


class MemStressGenerator:
    def __init__(self, seed, length):
        self.seed = seed
        self.length = length
        self.label_id = 0
        self.check_id = 0
        random.seed(seed)

    def next_label(self, prefix="L"):
        self.label_id += 1
        return f".{prefix}_{self.seed}_{self.label_id}"

    def emit_check_eq_reg(self, got_reg, expect_reg):
        self.check_id += 1
        ok = self.next_label("ok")
        return [
            f"  beq x{got_reg}, x{expect_reg}, {ok}",
            f"  # MISMATCH check #{self.check_id}: x{got_reg} != x{expect_reg}",
            f"  li x3, {self.check_id}",
            f"  RVTEST_FAIL",
            f"{ok}:",
        ]

    def emit_check_eq_imm(self, got_reg, value):
        self.check_id += 1
        ok = self.next_label("ok")
        return [
            f"  li x{CHECK_REG}, 0x{value & 0xFFFFFFFFFFFFFFFF:x}",
            f"  beq x{got_reg}, x{CHECK_REG}, {ok}",
            f"  li x3, {self.check_id}",
            f"  RVTEST_FAIL",
            f"{ok}:",
        ]

    def gen_waw_same_addr(self):
        """Multiple stores to the SAME address, verify last one wins."""
        off = random.choice(range(0, MAX_OFF, 8))
        n_stores = random.randint(3, 8)
        regs = random.sample(REGS, min(n_stores + 1, len(REGS)))
        store_regs = regs[:n_stores]
        rd = regs[n_stores] if len(regs) > n_stores else rand_reg_not(*store_regs)

        lines = [f"  # WAW: {n_stores} stores to same addr, last wins"]
        for sr in store_regs:
            lines.append(f"  sd x{sr}, {off}(x{MEM_BASE_REG})")
        lines.append(f"  ld x{rd}, {off}(x{MEM_BASE_REG})")
        lines.extend(self.emit_check_eq_reg(rd, store_regs[-1]))
        return lines

    def gen_raw_max_distance(self):
        """Store, then many unrelated ALU ops, then load — maximum RAW distance."""
        rs = rand_reg()
        rd = rand_reg_not(rs)
        off = random.choice(range(0, MAX_OFF, 8))

        lines = [
            f"  # RAW max distance",
            f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
        ]
        # Fill pipeline with ALU ops (don't clobber rs or rd)
        n_alu = random.randint(20, 60)
        for _ in range(n_alu):
            r1 = rand_reg_not(rs, rd)
            r2 = rand_reg_not(rs, rd)
            r3 = rand_reg_not(rs, rd)
            op = random.choice(["add", "sub", "xor", "or", "and", "sll", "srl"])
            lines.append(f"  {op} x{r3}, x{r1}, x{r2}")

        lines.append(f"  ld x{rd}, {off}(x{MEM_BASE_REG})")
        lines.extend(self.emit_check_eq_reg(rd, rs))
        return lines

    def gen_store_load_store_load(self):
        """Store A, load A, store A (different value), load A — verify both loads."""
        rs1 = rand_reg()
        rs2 = rand_reg_not(rs1)
        rd1 = rand_reg_not(rs1, rs2)
        rd2 = rand_reg_not(rs1, rs2, rd1)
        off = random.choice(range(0, MAX_OFF, 8))

        lines = [
            f"  # Store-load-store-load same addr",
            f"  sd x{rs1}, {off}(x{MEM_BASE_REG})",
            f"  ld x{rd1}, {off}(x{MEM_BASE_REG})",
            f"  sd x{rs2}, {off}(x{MEM_BASE_REG})",
            f"  ld x{rd2}, {off}(x{MEM_BASE_REG})",
        ]
        lines.extend(self.emit_check_eq_reg(rd1, rs1))
        lines.extend(self.emit_check_eq_reg(rd2, rs2))
        return lines

    def gen_width_mismatch_stress(self):
        """Store wide, load narrow from multiple sub-positions, verify all."""
        rs = rand_reg()
        off = random.choice(range(0, MAX_OFF, 8))

        lines = [
            f"  # Width mismatch stress",
            f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
        ]

        # Load each byte and verify
        for byte_off in range(8):
            rd = rand_reg_not(rs)
            lines.append(f"  lbu x{rd}, {off + byte_off}(x{MEM_BASE_REG})")
            # Expected: (rs >> (byte_off*8)) & 0xFF
            lines.append(f"  srli x{CHECK_REG}, x{rs}, {byte_off * 8}")
            lines.append(f"  andi x{CHECK_REG}, x{CHECK_REG}, 0xFF")
            lines.extend(self.emit_check_eq_reg(rd, CHECK_REG))

        return lines

    def gen_sb_capacity_test(self):
        """Fill store buffer to exact capacity boundaries (4, 8, 16, 32, 64 entries).

        The small config has SB=4, default has SB=32, linux has SB=64.
        """
        # Try to exceed even the largest SB
        n = random.choice([4, 5, 8, 9, 16, 17, 32, 33, 64, 65])
        n = min(n, MAX_OFF // 8)

        lines = [f"  # SB capacity test: {n} consecutive stores"]
        val_reg = rand_reg()

        for i in range(n):
            off = i * 8
            temp = rand_reg_not(val_reg)
            lines.append(f"  addi x{temp}, x{val_reg}, {i}")
            lines.append(f"  sd x{temp}, {off}(x{MEM_BASE_REG})")

        # Now verify ALL of them — by the time we read, some should have
        # drained from SB to cache/memory
        rd = rand_reg_not(val_reg)
        for i in range(n):
            off = i * 8
            lines.append(f"  ld x{rd}, {off}(x{MEM_BASE_REG})")
            lines.append(f"  addi x{CHECK_REG}, x{val_reg}, {i}")
            lines.extend(self.emit_check_eq_reg(rd, CHECK_REG))

        return lines

    def gen_store_across_branch_mispred(self):
        """Store on BOTH sides of a branch, then load and verify.

        The key stress: the branch predictor will likely mispredict,
        causing stores from the wrong path to be in the SB. They must
        be squashed correctly.
        """
        rs_taken = rand_reg()
        rs_not_taken = rand_reg_not(rs_taken)
        rd = rand_reg_not(rs_taken, rs_not_taken)
        off = random.choice(range(0, MAX_OFF, 8))

        # Use comparison regs with unpredictable relationship
        cmp1 = rand_reg_not(rs_taken, rs_not_taken, rd)
        cmp2 = rand_reg_not(rs_taken, rs_not_taken, rd, cmp1)

        taken_lbl = self.next_label("tk")
        end_lbl = self.next_label("end")

        lines = [
            f"  # Store across branch misprediction",
            f"  beq x{cmp1}, x{cmp2}, {taken_lbl}",
            # Not taken path
            f"  sd x{rs_not_taken}, {off}(x{MEM_BASE_REG})",
            f"  j {end_lbl}",
            f"{taken_lbl}:",
            f"  sd x{rs_taken}, {off}(x{MEM_BASE_REG})",
            f"{end_lbl}:",
            f"  ld x{rd}, {off}(x{MEM_BASE_REG})",
        ]

        # We need to verify against whichever path was actually taken
        # If cmp1 == cmp2, taken path executed, so rd should == rs_taken
        # If cmp1 != cmp2, not-taken path, so rd should == rs_not_taken
        check_taken = self.next_label("chk_tk")
        check_end = self.next_label("chk_end")
        lines.extend([
            f"  beq x{cmp1}, x{cmp2}, {check_taken}",
            # not-taken: check against rs_not_taken
        ])
        lines.extend(self.emit_check_eq_reg(rd, rs_not_taken))
        lines.extend([
            f"  j {check_end}",
            f"{check_taken}:",
        ])
        lines.extend(self.emit_check_eq_reg(rd, rs_taken))
        lines.append(f"{check_end}:")
        return lines

    def gen_triple_load_same_addr(self):
        """Three loads from the same address, verify all match.

        More aggressive than double-load: three chances to catch inconsistency.
        """
        rs = rand_reg()
        rd1 = rand_reg_not(rs)
        rd2 = rand_reg_not(rs, rd1)
        rd3 = rand_reg_not(rs, rd1, rd2)
        off = random.choice(range(0, MAX_OFF, 8))

        lines = [
            f"  # Triple load same addr",
            f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
        ]

        # Heavy ALU pressure between store and loads
        for _ in range(random.randint(5, 15)):
            r1 = rand_reg_not(rs, rd1, rd2, rd3)
            r2 = rand_reg_not(rs, rd1, rd2, rd3)
            r3 = rand_reg_not(rs, rd1, rd2, rd3)
            lines.append(f"  add x{r3}, x{r1}, x{r2}")

        lines.append(f"  ld x{rd1}, {off}(x{MEM_BASE_REG})")

        # More ALU between loads
        for _ in range(random.randint(3, 10)):
            r1 = rand_reg_not(rs, rd1, rd2, rd3)
            r2 = rand_reg_not(rs, rd1, rd2, rd3)
            r3 = rand_reg_not(rs, rd1, rd2, rd3)
            lines.append(f"  xor x{r3}, x{r1}, x{r2}")

        lines.append(f"  ld x{rd2}, {off}(x{MEM_BASE_REG})")

        for _ in range(random.randint(1, 5)):
            r1 = rand_reg_not(rs, rd1, rd2, rd3)
            r2 = rand_reg_not(rs, rd1, rd2, rd3)
            r3 = rand_reg_not(rs, rd1, rd2, rd3)
            lines.append(f"  sub x{r3}, x{r1}, x{r2}")

        lines.append(f"  ld x{rd3}, {off}(x{MEM_BASE_REG})")

        # All three must match rs and each other
        lines.extend(self.emit_check_eq_reg(rd1, rs))
        lines.extend(self.emit_check_eq_reg(rd2, rs))
        lines.extend(self.emit_check_eq_reg(rd3, rs))
        return lines

    def gen_store_chain_verify(self):
        """Store a value, load it, use loaded value to compute next store, repeat.

        Creates a data dependency chain through memory — stresses forwarding
        under dependency pressure.
        """
        off1 = random.choice(range(0, MAX_OFF - 32, 8))
        off2 = off1 + 8
        off3 = off1 + 16

        rs = rand_reg()
        r1 = rand_reg_not(rs)
        r2 = rand_reg_not(rs, r1)
        r3 = rand_reg_not(rs, r1, r2)

        lines = [
            f"  # Memory dependency chain",
            f"  sd x{rs}, {off1}(x{MEM_BASE_REG})",
            f"  ld x{r1}, {off1}(x{MEM_BASE_REG})",
            f"  addi x{r1}, x{r1}, 1",
            f"  sd x{r1}, {off2}(x{MEM_BASE_REG})",
            f"  ld x{r2}, {off2}(x{MEM_BASE_REG})",
            f"  addi x{r2}, x{r2}, 1",
            f"  sd x{r2}, {off3}(x{MEM_BASE_REG})",
            f"  ld x{r3}, {off3}(x{MEM_BASE_REG})",
        ]

        # r3 should == rs + 2
        lines.append(f"  addi x{CHECK_REG}, x{rs}, 2")
        lines.extend(self.emit_check_eq_reg(r3, CHECK_REG))
        return lines

    def gen_amo_store_race(self):
        """Store to addr, then AMO on same addr, verify AMO returns store's value."""
        rs = rand_reg()
        rd = rand_reg_not(rs)
        amo_val = rand_reg_not(rs, rd)
        off = random.choice(range(0, MAX_OFF, 8))
        addr_reg = rand_reg_not(rs, rd, amo_val)

        lines = [
            f"  # AMO after store (same addr)",
            f"  addi x{addr_reg}, x{MEM_BASE_REG}, {off}",
            f"  sd x{rs}, 0(x{addr_reg})",
        ]
        # Some ALU noise
        for _ in range(random.randint(0, 5)):
            r1 = rand_reg_not(rs, rd, amo_val, addr_reg)
            r2 = rand_reg_not(rs, rd, amo_val, addr_reg)
            r3 = rand_reg_not(rs, rd, amo_val, addr_reg)
            lines.append(f"  add x{r3}, x{r1}, x{r2}")

        amo_op = random.choice(["amoadd.d", "amoxor.d", "amoor.d", "amoand.d"])
        lines.append(f"  {amo_op} x{rd}, x{amo_val}, (x{addr_reg})")
        # AMO returns old value which should be rs
        lines.extend(self.emit_check_eq_reg(rd, rs))

        return lines

    def gen_overlapping_width_stores(self):
        """Store byte, halfword, word to overlapping region, then load and verify.

        The last store to each byte position wins.
        """
        off = random.choice(range(0, MAX_OFF - 16, 8))
        rs = rand_reg()
        rd = rand_reg_not(rs)

        # Store a doubleword, then overwrite with a word at same address
        # The word should replace the lower 4 bytes
        rs2 = rand_reg_not(rs, rd)
        lines = [
            f"  # Overlapping width stores",
            f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
            f"  sw x{rs2}, {off}(x{MEM_BASE_REG})",  # overwrite lower 4 bytes
            f"  ld x{rd}, {off}(x{MEM_BASE_REG})",
        ]

        # Expected: upper 4 bytes from rs, lower 4 bytes from rs2
        # Build expected = (rs & 0xFFFFFFFF00000000) | (rs2 & 0xFFFFFFFF)
        t1 = rand_reg_not(rs, rs2, rd)
        t2 = rand_reg_not(rs, rs2, rd, t1)
        lines.extend([
            f"  # Compute expected: upper(rs) | lower(rs2)",
            f"  srli x{t1}, x{rs}, 32",
            f"  slli x{t1}, x{t1}, 32",      # upper 32 bits of rs
            f"  slli x{t2}, x{rs2}, 32",
            f"  srli x{t2}, x{t2}, 32",       # lower 32 bits of rs2
            f"  or x{CHECK_REG}, x{t1}, x{t2}",
        ])
        lines.extend(self.emit_check_eq_reg(rd, CHECK_REG))
        return lines

    def gen_rapid_same_addr_different_widths(self):
        """Rapidly alternate store/load of different widths to same address."""
        off = random.choice(range(0, MAX_OFF, 8))
        rs = rand_reg()
        rd = rand_reg_not(rs)

        lines = [f"  # Rapid width alternation at same address"]

        # Store doubleword
        lines.append(f"  sd x{rs}, {off}(x{MEM_BASE_REG})")
        # Load word (lower 4 bytes)
        lines.append(f"  lwu x{rd}, {off}(x{MEM_BASE_REG})")
        lines.append(f"  slli x{CHECK_REG}, x{rs}, 32")
        lines.append(f"  srli x{CHECK_REG}, x{CHECK_REG}, 32")
        lines.extend(self.emit_check_eq_reg(rd, CHECK_REG))

        # Load halfword (lower 2 bytes)
        rd2 = rand_reg_not(rs, rd)
        lines.append(f"  lhu x{rd2}, {off}(x{MEM_BASE_REG})")
        lines.append(f"  slli x{CHECK_REG}, x{rs}, 48")
        lines.append(f"  srli x{CHECK_REG}, x{CHECK_REG}, 48")
        lines.extend(self.emit_check_eq_reg(rd2, CHECK_REG))

        # Load byte (lowest byte)
        rd3 = rand_reg_not(rs, rd, rd2)
        lines.append(f"  lbu x{rd3}, {off}(x{MEM_BASE_REG})")
        lines.append(f"  andi x{CHECK_REG}, x{rs}, 0xFF")
        lines.extend(self.emit_check_eq_reg(rd3, CHECK_REG))

        return lines

    def gen_lr_sc_verify(self):
        """LR/SC to a clean address — SC should succeed."""
        rs = rand_reg()
        rd = rand_reg_not(rs)
        sc_result = rand_reg_not(rs, rd)
        off = random.choice(range(0, MAX_OFF, 8))
        addr_reg = rand_reg_not(rs, rd, sc_result)

        lines = [
            f"  # LR/SC verify",
            f"  addi x{addr_reg}, x{MEM_BASE_REG}, {off}",
            f"  sd x{rs}, 0(x{addr_reg})",
            f"  lr.d x{rd}, (x{addr_reg})",
        ]
        # rd should equal rs
        lines.extend(self.emit_check_eq_reg(rd, rs))

        # SC with new value
        new_val = rand_reg_not(rs, rd, sc_result, addr_reg)
        lines.append(f"  sc.d x{sc_result}, x{new_val}, (x{addr_reg})")
        # SC result should be 0 (success)
        lines.extend(self.emit_check_eq_imm(sc_result, 0))

        # Verify memory has new value
        rd2 = rand_reg_not(new_val)
        lines.append(f"  ld x{rd2}, 0(x{addr_reg})")
        lines.extend(self.emit_check_eq_reg(rd2, new_val))

        return lines

    def generate(self):
        lines = []
        lines.append(f"# Memory stress test -- seed {self.seed}, length {self.length}")
        lines.append('#include "riscv_test.h"')
        lines.append('#include "test_macros.h"')
        lines.append("")
        lines.append("RVTEST_RV64U")
        lines.append("RVTEST_CODE_BEGIN")
        lines.append("")

        # Initialize registers
        lines.append("  # Initialize registers with distinct values")
        rng_init = random.Random(self.seed + 0xDEAD)
        for r in REGS:
            val = rng_init.randint(1, (1 << 63) - 1)
            lines.append(f"  li x{r}, 0x{val:016x}")
        lines.append("")

        lines.append("  la x4, scratch_data")
        lines.append("")

        # Pre-init scratch memory
        lines.append("  li x31, 0xDEADBEEFCAFEBABE")
        for off in range(0, 2048, 8):
            lines.append(f"  sd x31, {off}(x4)")
        lines.append("")

        # Re-init x31
        rng_init2 = random.Random(self.seed + 0xDEAD)
        for r in REGS:
            v = rng_init2.randint(1, (1 << 63) - 1)
            if r == CHECK_REG:
                lines.append(f"  li x{CHECK_REG}, 0x{v:016x}")
                break
        lines.append("")

        generators = [
            (15, self.gen_waw_same_addr),
            (10, self.gen_raw_max_distance),
            (15, self.gen_store_load_store_load),
            (10, self.gen_width_mismatch_stress),
            (8, self.gen_sb_capacity_test),
            (10, self.gen_store_across_branch_mispred),
            (12, self.gen_triple_load_same_addr),
            (8, self.gen_store_chain_verify),
            (5, self.gen_amo_store_race),
            (7, self.gen_overlapping_width_stores),
            (5, self.gen_rapid_same_addr_different_widths),
            (5, self.gen_lr_sc_verify),
        ]
        total_weight = sum(w for w, _ in generators)

        i = 0
        while i < self.length:
            roll = random.random() * total_weight
            cumulative = 0
            for weight, gen_fn in generators:
                cumulative += weight
                if roll < cumulative:
                    seq = gen_fn()
                    lines.extend(seq)
                    i += len(seq)
                    break

        lines.append("")
        lines.append("  RVTEST_PASS")
        lines.append("")
        lines.append("RVTEST_CODE_END")
        lines.append("")
        lines.append("  .data")
        lines.append("RVTEST_DATA_BEGIN")
        lines.append("")
        lines.append("  .align 4")
        lines.append("scratch_data:")
        lines.append("  .fill 4096, 1, 0")
        lines.append("")
        lines.append("RVTEST_DATA_END")

        return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description="Generate memory stress torture tests")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--count", type=int, default=500)
    ap.add_argument("--length", type=int, default=500)
    ap.add_argument("--outdir", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "generated"))
    args = ap.parse_args()

    os.makedirs(args.outdir, exist_ok=True)

    for i in range(args.count):
        seed = args.seed + i
        gen = MemStressGenerator(seed=seed, length=args.length)
        code = gen.generate()
        path = os.path.join(args.outdir, f"memstress_{seed:06d}.S")
        with open(path, "w") as f:
            f.write(code)

    print(f"Generated {args.count} memory stress tests in {args.outdir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
