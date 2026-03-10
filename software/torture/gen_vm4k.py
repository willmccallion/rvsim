#!/usr/bin/env python3
"""Generate VM tests with 4KB page table mappings (3-level Sv39).

Unlike vmstress (which uses gigapage identity map), these tests set up
full 3-level Sv39 page tables with 4KB pages. This forces the PTW to
do 3 bus reads per walk, which is much more realistic and stresses
the PTW + store buffer interaction harder.

Uses M-mode with MPRV+MPP=S, same as vmstress.

Memory layout:
  - Code region:     identity-mapped via gigapage (L2 entry)
  - Scratch region:  mapped via 4KB pages (L2 → L1 → L0)
  - Page tables:     in identity-mapped region (accessible from M-mode)

The 4KB scratch pages are mapped at VA 0x80100000-0x80100FFF (4KB).
Physical backing is at scratch_phys in the data section.

Usage:
    python gen_vm4k.py --seed 0 --count 500
"""

import argparse
import os
import random
import sys

PTE_V = 0x001
PTE_R = 0x002
PTE_W = 0x004
PTE_X = 0x008
PTE_U = 0x010
PTE_G = 0x020
PTE_A = 0x040
PTE_D = 0x080
PTE_PPN_SHIFT = 10

SATP_MODE_SV39 = 8
SATP_MODE_SHIFT = 60


class Vm4kGenerator:
    def __init__(self, seed, num_patterns):
        self.seed = seed
        self.num_patterns = num_patterns
        self.label_id = 0
        self.check_id = 0
        random.seed(seed)

    def next_label(self, prefix="L"):
        self.label_id += 1
        return f".{prefix}_{self.seed}_{self.label_id}"

    def _rand_reg(self, *exclude):
        pool = [r for r in range(5, 28) if r not in exclude]
        return random.choice(pool)

    def _emit_check(self, got_reg, expect_reg):
        self.check_id += 1
        ok = self.next_label("ok")
        return [
            f"  beq x{got_reg}, x{expect_reg}, {ok}",
            f"  li x3, {self.check_id}",
            f"  RVTEST_FAIL",
            f"{ok}:",
        ]

    def generate(self):
        lines = []
        lines.append(f"# VM 4KB page test -- seed {self.seed}")
        lines.append('#include "riscv_test.h"')
        lines.append('#include "test_macros.h"')
        lines.append("")
        lines.append("RVTEST_RV64M")
        lines.append("RVTEST_CODE_BEGIN")
        lines.append("")

        # ── Setup Sv39 page tables ──
        # L2[2] = gigapage for code (identity map 0x80000000)
        # L2[2] also covers our page tables (they're in this region)
        # BUT: we want 4KB pages for scratch, so we need a DIFFERENT VA range.
        #
        # Strategy: Use L2[2] as gigapage for code+data (identity),
        # and have a 4KB-mapped scratch at the SAME physical region.
        #
        # Actually simpler: just use the gigapage for everything and also
        # set up a separate VA range via 4KB pages for the scratch.
        # L2[3] → L1 table → L0 table → 4KB pages
        # VA 0xC0000000 maps to scratch_phys via 4KB pages.
        #
        # This way PTW does 3 reads: L2[3] → L1[0] → L0[0] → PTE

        lines.extend([
            "  # ── Setup Sv39 with 3-level page table for scratch ──",
            "",
            "  # First, write the page table entries at runtime.",
            "  # This ensures the stores go through the store buffer path.",
            "",
            "  # L2 root table setup:",
            "  # L2[2] = gigapage for 0x80000000 (identity, code+data)",
            "  # L2[3] = pointer to L1 table (for 0xC0000000 4KB-mapped scratch)",
            "  la t0, page_table_l2",
            "",
            "  # L2[2]: gigapage identity map for code region",
            f"  li t1, ({0x80000 << PTE_PPN_SHIFT} | {PTE_V | PTE_R | PTE_W | PTE_X | PTE_U | PTE_A | PTE_D})",
            "  sd t1, 16(t0)     # L2[2]",
            "",
            "  # L2[3]: pointer to L1 table (non-leaf, V only)",
            "  la t1, page_table_l1",
            f"  srli t2, t1, 12",
            f"  slli t2, t2, {PTE_PPN_SHIFT}",
            f"  ori t2, t2, {PTE_V}",
            "  sd t2, 24(t0)     # L2[3]",
            "",
            "  # L1[0]: pointer to L0 table",
            "  la t0, page_table_l1",
            "  la t1, page_table_l0",
            f"  srli t2, t1, 12",
            f"  slli t2, t2, {PTE_PPN_SHIFT}",
            f"  ori t2, t2, {PTE_V}",
            "  sd t2, 0(t0)      # L1[0]",
            "",
            "  # L0[0]: 4KB leaf page mapping to scratch_phys",
            "  la t0, page_table_l0",
            "  la t1, scratch_phys",
            f"  srli t2, t1, 12",
            f"  slli t2, t2, {PTE_PPN_SHIFT}",
            f"  li t3, {PTE_V | PTE_R | PTE_W | PTE_X | PTE_U | PTE_A | PTE_D}",
            "  or t2, t2, t3",
            "  sd t2, 0(t0)      # L0[0]",
            "",
            "  # L0[1]: second 4KB page",
            "  la t1, scratch_phys2",
            f"  srli t2, t1, 12",
            f"  slli t2, t2, {PTE_PPN_SHIFT}",
            f"  li t3, {PTE_V | PTE_R | PTE_W | PTE_X | PTE_U | PTE_A | PTE_D}",
            "  or t2, t2, t3",
            "  sd t2, 8(t0)      # L0[1]",
            "",
            "  # Activate Sv39",
            f"  li t0, {SATP_MODE_SV39 << SATP_MODE_SHIFT:#x}",
            "  la t1, page_table_l2",
            "  srli t1, t1, 12",
            "  or t0, t0, t1",
            "  csrw satp, t0",
            "  sfence.vma",
            "",
            "  # Enable MPRV with MPP=S and SUM",
            "  li t0, (1 << 17) | (1 << 11)  # MPRV | MPP=S",
            "  csrs mstatus, t0",
            "  li t0, (1 << 18)               # SUM",
            "  csrs mstatus, t0",
            "",
        ])

        # Initialize registers
        lines.append("  # Initialize registers")
        rng_init = random.Random(self.seed + 0xBEEF)
        for r in range(5, 28):
            val = rng_init.randint(1, (1 << 63) - 1)
            lines.append(f"  li x{r}, 0x{val:016x}")
        lines.append("")

        # x4 (tp) = VA of scratch area = 0xC0000000
        lines.append("  # x4 = VA of 4KB-mapped scratch (0xC0000000)")
        lines.append("  li x4, 0xC0000000")
        lines.append("")

        # Pre-init scratch via VM
        lines.append("  li x31, 0xA5A5A5A5A5A5A5A5")
        for off in range(0, 2048, 8):
            lines.append(f"  sd x31, {off}(x4)")
        lines.append("")

        # Restore x31
        val31 = random.Random(self.seed + 0xBEEF + 26).randint(1, (1 << 63) - 1)
        lines.append(f"  li x31, 0x{val31:016x}")
        lines.append("")

        # Generate test patterns (same as vmstress but through 4KB pages)
        generators = [
            (20, self.gen_store_load),
            (20, self.gen_double_load),
            (15, self.gen_sfence_store_load),
            (10, self.gen_store_flood),
            (10, self.gen_width_mismatch),
            (10, self.gen_interleaved),
            (10, self.gen_branch_store_load),
            (5, self.gen_pte_rewrite_access),
        ]
        total_weight = sum(w for w, _ in generators)

        i = 0
        while i < self.num_patterns:
            roll = random.random() * total_weight
            cumulative = 0
            for weight, gen_fn in generators:
                cumulative += weight
                if roll < cumulative:
                    seq = gen_fn()
                    lines.extend(seq)
                    lines.append("")
                    i += 1
                    break

        # Cleanup
        lines.extend([
            "  li t0, (1 << 17)",
            "  csrc mstatus, t0",
            "",
            "  RVTEST_PASS",
            "",
            "  TEST_PASSFAIL",
            "",
            "  .align 2",
            "  .global mtvec_handler",
            "mtvec_handler:",
            "  csrr t0, mcause",
            "  csrr t1, mepc",
            "  csrr t2, mtval",
            "  li gp, 0xFF",
            "  RVTEST_FAIL",
            "",
            "RVTEST_CODE_END",
            "",
        ])

        # Data section
        lines.extend([
            "  .data",
            "RVTEST_DATA_BEGIN",
            "",
            "  TEST_DATA",
            "",
            "# Page tables (each 4KB aligned)",
            ".align 12",
            "page_table_l2:",
            ".fill 512, 8, 0",
            "",
            ".align 12",
            "page_table_l1:",
            ".fill 512, 8, 0",
            "",
            ".align 12",
            "page_table_l0:",
            ".fill 512, 8, 0",
            "",
            "# Physical backing for scratch pages (4KB aligned)",
            ".align 12",
            "scratch_phys:",
            ".fill 4096, 1, 0",
            "",
            ".align 12",
            "scratch_phys2:",
            ".fill 4096, 1, 0",
            "",
            "RVTEST_DATA_END",
        ])

        return "\n".join(lines)

    def gen_store_load(self):
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))
        lines = [
            f"  sd x{rs}, {off}(x4)",
            f"  ld x{rd}, {off}(x4)",
        ]
        lines.extend(self._emit_check(rd, rs))
        return lines

    def gen_double_load(self):
        rs = self._rand_reg()
        rd1 = self._rand_reg(rs)
        rd2 = self._rand_reg(rs, rd1)
        off = random.choice(range(0, 2040, 8))

        lines = [
            f"  # Double load via 4KB pages",
            f"  sd x{rs}, {off}(x4)",
        ]
        for _ in range(random.randint(2, 10)):
            r1 = self._rand_reg(rs, rd1, rd2)
            r2 = self._rand_reg(rs, rd1, rd2)
            r3 = self._rand_reg(rs, rd1, rd2)
            lines.append(f"  add x{r3}, x{r1}, x{r2}")

        lines.append(f"  ld x{rd1}, {off}(x4)")
        for _ in range(random.randint(0, 6)):
            r1 = self._rand_reg(rs, rd1, rd2)
            r2 = self._rand_reg(rs, rd1, rd2)
            r3 = self._rand_reg(rs, rd1, rd2)
            lines.append(f"  xor x{r3}, x{r1}, x{r2}")
        lines.append(f"  ld x{rd2}, {off}(x4)")
        lines.extend(self._emit_check(rd1, rd2))
        lines.extend(self._emit_check(rd1, rs))
        return lines

    def gen_sfence_store_load(self):
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))
        lines = [
            f"  sd x{rs}, {off}(x4)",
            f"  sfence.vma",
            f"  ld x{rd}, {off}(x4)",
        ]
        lines.extend(self._emit_check(rd, rs))
        return lines

    def gen_store_flood(self):
        n = random.randint(8, 32)
        n = min(n, 255)
        val_reg = self._rand_reg()
        rd = self._rand_reg(val_reg)

        lines = [f"  # Store flood via 4KB pages: {n} stores"]
        for i in range(n):
            off = i * 8
            temp = self._rand_reg(val_reg, rd)
            lines.append(f"  addi x{temp}, x{val_reg}, {i}")
            lines.append(f"  sd x{temp}, {off}(x4)")

        lines.append(f"  sfence.vma")
        for i in range(n):
            off = i * 8
            lines.append(f"  ld x{rd}, {off}(x4)")
            lines.append(f"  addi x31, x{val_reg}, {i}")
            lines.extend(self._emit_check(rd, 31))
        return lines

    def gen_width_mismatch(self):
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))
        lines = [
            f"  sd x{rs}, {off}(x4)",
        ]
        for byte_off in range(8):
            lines.append(f"  lbu x{rd}, {off + byte_off}(x4)")
            lines.append(f"  srli x31, x{rs}, {byte_off * 8}")
            lines.append(f"  andi x31, x31, 0xFF")
            lines.extend(self._emit_check(rd, 31))
        return lines

    def gen_interleaved(self):
        n = random.randint(3, 6)
        regs = random.sample(range(5, 28), min(n * 2, 23))
        store_regs = regs[:n]
        load_regs = regs[n:n*2]
        offsets = [i * 8 for i in range(n)]
        protected = set(store_regs + load_regs)

        lines = [f"  # Interleaved via 4KB pages: {n} pairs"]
        for i in range(n):
            lines.append(f"  sd x{store_regs[i]}, {offsets[i]}(x4)")
        for i in range(n):
            if i > 0 and random.random() < 0.5:
                r1 = self._rand_reg(*protected)
                r2 = self._rand_reg(*protected, r1)
                lines.append(f"  xor x{r1}, x{r1}, x{r2}")
            lines.append(f"  ld x{load_regs[i]}, {offsets[i]}(x4)")
        for i in range(n):
            lines.extend(self._emit_check(load_regs[i], store_regs[i]))
        return lines

    def gen_branch_store_load(self):
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))
        cmp1 = self._rand_reg(rs, rd)
        cmp2 = self._rand_reg(rs, rd, cmp1)
        taken = self.next_label("tk")
        end = self.next_label("end")

        lines = [
            f"  sd x{rs}, {off}(x4)",
            f"  beq x{cmp1}, x{cmp2}, {taken}",
        ]
        for _ in range(random.randint(2, 5)):
            t = self._rand_reg(rs, rd)
            t2 = self._rand_reg(rs, rd, t)
            lines.append(f"  add x{t}, x{t}, x{t2}")
        lines.extend([
            f"  j {end}",
            f"{taken}:",
        ])
        for _ in range(random.randint(2, 5)):
            t = self._rand_reg(rs, rd)
            t2 = self._rand_reg(rs, rd, t)
            lines.append(f"  sub x{t}, x{t}, x{t2}")
        lines.extend([
            f"{end}:",
            f"  ld x{rd}, {off}(x4)",
        ])
        lines.extend(self._emit_check(rd, rs))
        return lines

    def gen_pte_rewrite_access(self):
        """Rewrite L0 PTE (same value), SFENCE, access 4KB page.
        Forces PTW to re-walk 3 levels after store buffer drains.
        """
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))

        lines = [
            f"  # PTE rewrite + SFENCE + 4KB access",
            # Store some data first
            f"  sd x{rs}, {off}(x4)",
            # Now temporarily disable MPRV to access PT directly
            f"  li t0, (1 << 17)",
            f"  csrc mstatus, t0",   # disable MPRV
            # Read current L0 PTE and write it back (same value)
            f"  la t1, page_table_l0",
            f"  ld t2, 0(t1)",
            f"  sd t2, 0(t1)",
            # Re-enable MPRV
            f"  li t0, (1 << 17)",
            f"  csrs mstatus, t0",
            # SFENCE to force TLB refill → PTW re-walk
            f"  sfence.vma",
            # Load back through 4KB page — PTW must walk 3 levels
            f"  ld x{rd}, {off}(x4)",
        ]
        lines.extend(self._emit_check(rd, rs))
        return lines


def main():
    ap = argparse.ArgumentParser(description="Generate 4KB page VM tests")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--count", type=int, default=500)
    ap.add_argument("--patterns", type=int, default=100)
    ap.add_argument("--outdir", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "generated"))
    args = ap.parse_args()

    os.makedirs(args.outdir, exist_ok=True)
    for i in range(args.count):
        seed = args.seed + i
        gen = Vm4kGenerator(seed=seed, num_patterns=args.patterns)
        code = gen.generate()
        path = os.path.join(args.outdir, f"vm4k_{seed:06d}.S")
        with open(path, "w") as f:
            f.write(code)
    print(f"Generated {args.count} 4KB page VM tests in {args.outdir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
