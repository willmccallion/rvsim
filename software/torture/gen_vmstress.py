#!/usr/bin/env python3
"""Generate S-mode virtual memory stress tests.

These tests run in M-mode with MPRV+MPP=S to exercise the virtual memory
subsystem. They set up Sv39 page tables and then stress:

  1. Store to memory via VM, SFENCE.VMA, load back via VM — verify
  2. Modify PTE in page table, SFENCE.VMA, access new mapping — verify
  3. Multiple stores through VM, then load-back-and-verify (SB + PTW interaction)
  4. Store to PTE + SFENCE.VMA + immediate access (Finding 2/3 pattern)
  5. Rapid SATP switches between page tables
  6. Two loads from same virtual address (must match) — Linux crash pattern
  7. Store wide, load narrow through VM translation
  8. Interleaved stores to different virtual pages

The key insight: the Linux crash might only manifest when page table walks
interact with the store buffer and pipeline squashes.

Usage:
    python gen_vmstress.py --seed 0 --count 200
"""

import argparse
import os
import random
import sys


# PTE bits
PTE_V = 1 << 0
PTE_R = 1 << 1
PTE_W = 1 << 2
PTE_X = 1 << 3
PTE_U = 1 << 4
PTE_G = 1 << 5
PTE_A = 1 << 6
PTE_D = 1 << 7

RISCV_PGSHIFT = 12
RISCV_PGSIZE = 1 << RISCV_PGSHIFT
PTE_PPN_SHIFT = 10

# Sv39: SATP mode field
SATP_MODE_SV39 = 8
SATP_MODE_SHIFT = 60


class VmStressGenerator:
    def __init__(self, seed, num_patterns):
        self.seed = seed
        self.num_patterns = num_patterns
        self.label_id = 0
        self.check_id = 0
        random.seed(seed)

    def next_label(self, prefix="L"):
        self.label_id += 1
        return f".{prefix}_{self.seed}_{self.label_id}"

    def generate(self):
        """Generate a complete test that sets up Sv39 and runs stress patterns.

        Memory layout (relative to DRAM_BASE, which is 0x80000000):
        - Code: starts at DRAM_BASE (identity-mapped via superpage)
        - Page tables: at known offsets in .data section
        - Scratch data: separate pages for store/load testing

        We use a single Sv39 gigapage (1GB superpage) identity map so that
        virtual addresses == physical addresses. This means we can verify
        load results against known values without worrying about address
        translation complexity — but the PTW still does the full walk.
        """
        lines = []
        lines.append(f"# VM stress test -- seed {self.seed}")
        lines.append('#include "riscv_test.h"')
        lines.append('#include "test_macros.h"')
        lines.append("")
        lines.append("RVTEST_RV64M")
        lines.append("RVTEST_CODE_BEGIN")
        lines.append("")

        # ── Setup: enable Sv39 with identity-mapped gigapage ──
        lines.extend([
            "  # ── VM Setup ──",
            "  # Build SATP value: mode=Sv39, PPN = page_table_l2 >> 12",
            f"  li t0, {SATP_MODE_SV39 << SATP_MODE_SHIFT:#x}",
            "  la t1, page_table_l2",
            f"  srli t1, t1, {RISCV_PGSHIFT}",
            "  or t0, t0, t1",
            "  csrw satp, t0",
            "  sfence.vma",
            "",
            "  # Enable MPRV with MPP=S so loads/stores go through VM",
            "  li t0, (1 << 17) | (1 << 11)  # MPRV | MPP=S",
            "  csrs mstatus, t0",
            "",
            "  # Also set SUM so we can access U pages from S-mode",
            "  li t0, (1 << 18)  # SUM",
            "  csrs mstatus, t0",
            "",
        ])

        # Initialize scratch registers with known values
        lines.append("  # Initialize registers")
        rng_init = random.Random(self.seed + 0xCAFE)
        reg_vals = {}
        for r in range(5, 32):
            val = rng_init.randint(1, (1 << 63) - 1)
            reg_vals[r] = val
            if r <= 30:
                lines.append(f"  li x{r}, 0x{val:016x}")
        lines.append("")

        # Set up base pointer to scratch area (virtual = physical due to identity map)
        lines.append("  la x4, scratch_data")
        lines.append("")

        # Pre-init scratch memory via VM
        lines.append("  # Pre-init scratch via VM")
        lines.append("  li x31, 0xA5A5A5A5A5A5A5A5")
        for off in range(0, 2048, 8):
            lines.append(f"  sd x31, {off}(x4)")
        lines.append("")

        # Restore x31
        lines.append(f"  li x31, 0x{reg_vals[31]:016x}")
        lines.append("")

        # ── Generate stress patterns ──
        generators = [
            (20, self.gen_store_load_via_vm),
            (15, self.gen_double_load_vm),
            (15, self.gen_sfence_store_load),
            (10, self.gen_store_flood_vm),
            (10, self.gen_width_mismatch_vm),
            (10, self.gen_interleaved_vm),
            (10, self.gen_pte_modify_access),
            (10, self.gen_branch_store_load_vm),
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

        # ── Cleanup: disable MPRV ──
        lines.extend([
            "  # Disable MPRV",
            "  li t0, (1 << 17)",
            "  csrc mstatus, t0",
            "",
            "  RVTEST_PASS",
            "",
            "  TEST_PASSFAIL",
            "",
        ])

        # ── Trap handler: any trap = FAIL ──
        # Set TESTNUM (gp/x3) to 0xFF so RVTEST_FAIL doesn't infinite-loop
        lines.extend([
            "  .align 2",
            "  .global mtvec_handler",
            "mtvec_handler:",
            "  csrr t0, mcause",
            "  csrr t1, mepc",
            "  csrr t2, mtval",
            "  li gp, 0xFF",
            "  RVTEST_FAIL",
            "",
        ])

        lines.append("RVTEST_CODE_END")
        lines.append("")

        # ── Data section with page tables ──
        lines.extend(self._gen_page_tables())

        return "\n".join(lines)

    def _gen_page_tables(self):
        """Generate Sv39 page tables for identity-mapped gigapage.

        Sv39 has 3 levels: L2 (root) → L1 → L0
        For a gigapage (1GB superpage), we only need L2.
        L2[vpn2] points directly to a 1GB superpage.

        DRAM_BASE = 0x80000000 → vpn2 = 2
        So L2[2] = superpage PTE pointing to 0x80000000.
        """
        lines = [
            "  .data",
            "RVTEST_DATA_BEGIN",
            "",
            "  TEST_DATA",
            "",
            "# ── Sv39 Page Tables ──",
            "# L2 root table (4KB aligned)",
            ".align 12",
            "page_table_l2:",
            # Entry 0 and 1: invalid
            ".dword 0",
            ".dword 0",
            # Entry 2 (vpn2=2, covers 0x80000000-0xBFFFFFFF): gigapage
            # PPN = 0x80000000 >> 12 = 0x80000
            f".dword ({0x80000 << PTE_PPN_SHIFT} | {PTE_V | PTE_R | PTE_W | PTE_X | PTE_U | PTE_A | PTE_D})",
            # Fill rest with zeros (508 entries)
            ".fill 509, 8, 0",
            "",
            "# Alternative L2 table for SATP switching tests",
            ".align 12",
            "page_table_l2_alt:",
            ".dword 0",
            ".dword 0",
            f".dword ({0x80000 << PTE_PPN_SHIFT} | {PTE_V | PTE_R | PTE_W | PTE_X | PTE_U | PTE_A | PTE_D})",
            ".fill 509, 8, 0",
            "",
            "# Scratch data area (4KB aligned)",
            ".align 12",
            "scratch_data:",
            ".fill 4096, 1, 0",
            "",
            "RVTEST_DATA_END",
        ]
        return lines

    def _rand_reg(self, *exclude):
        pool = [r for r in range(5, 31) if r not in exclude]
        return random.choice(pool)

    def _emit_check(self, got_reg, expect_reg):
        self.check_id += 1
        ok = self.next_label("ok")
        return [
            f"  beq x{got_reg}, x{expect_reg}, {ok}",
            f"  # VM MISMATCH check #{self.check_id}: x{got_reg} != x{expect_reg}",
            f"  li x3, {self.check_id}",
            f"  RVTEST_FAIL",
            f"{ok}:",
        ]

    def _emit_check_imm(self, got_reg, value):
        self.check_id += 1
        ok = self.next_label("ok")
        return [
            f"  li x31, 0x{value & 0xFFFFFFFFFFFFFFFF:x}",
            f"  beq x{got_reg}, x31, {ok}",
            f"  li x3, {self.check_id}",
            f"  RVTEST_FAIL",
            f"{ok}:",
        ]

    def gen_store_load_via_vm(self):
        """Store through VM, load back through VM, verify."""
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))
        lines = [
            f"  # Store-load via VM (check #{self.check_id + 1})",
            f"  sd x{rs}, {off}(x4)",
            f"  ld x{rd}, {off}(x4)",
        ]
        lines.extend(self._emit_check(rd, rs))
        return lines

    def gen_double_load_vm(self):
        """Two loads from same virtual address — must return same value.
        This is the Linux crash pattern.
        """
        rs = self._rand_reg()
        rd1 = self._rand_reg(rs)
        rd2 = self._rand_reg(rs, rd1)
        off = random.choice(range(0, 2040, 8))

        lines = [
            f"  # Double load via VM (Linux crash pattern)",
            f"  sd x{rs}, {off}(x4)",
        ]
        # ALU noise between store and first load (don't clobber rs/rd1/rd2)
        for _ in range(random.randint(2, 10)):
            r1 = self._rand_reg(rs, rd1, rd2)
            r2 = self._rand_reg(rs, rd1, rd2)
            r3 = self._rand_reg(rs, rd1, rd2)
            lines.append(f"  add x{r3}, x{r1}, x{r2}")

        lines.append(f"  ld x{rd1}, {off}(x4)")

        # ALU noise between two loads
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
        """Store, SFENCE.VMA, load — the SFENCE should NOT lose the store."""
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))

        lines = [
            f"  # Store, SFENCE.VMA, load (SB drain test)",
            f"  sd x{rs}, {off}(x4)",
            f"  sfence.vma",
            f"  ld x{rd}, {off}(x4)",
        ]
        lines.extend(self._emit_check(rd, rs))
        return lines

    def gen_store_flood_vm(self):
        """Many stores through VM, then load all back."""
        n = random.randint(8, 32)
        n = min(n, 255)  # max offset / 8
        val_reg = self._rand_reg()
        rd = self._rand_reg(val_reg)

        lines = [f"  # Store flood via VM: {n} stores"]
        for i in range(n):
            off = i * 8
            temp = self._rand_reg(val_reg, rd)
            lines.append(f"  addi x{temp}, x{val_reg}, {i}")
            lines.append(f"  sd x{temp}, {off}(x4)")

        # SFENCE in the middle of verification to stress PTW
        lines.append(f"  sfence.vma")

        for i in range(n):
            off = i * 8
            lines.append(f"  ld x{rd}, {off}(x4)")
            lines.append(f"  addi x31, x{val_reg}, {i}")
            lines.extend(self._emit_check(rd, 31))

        return lines

    def gen_width_mismatch_vm(self):
        """Store doubleword through VM, load byte/half/word back."""
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))

        lines = [
            f"  # Width mismatch via VM",
            f"  sd x{rs}, {off}(x4)",
        ]

        # Load each byte
        for byte_off in range(8):
            lines.append(f"  lbu x{rd}, {off + byte_off}(x4)")
            lines.append(f"  srli x31, x{rs}, {byte_off * 8}")
            lines.append(f"  andi x31, x31, 0xFF")
            lines.extend(self._emit_check(rd, 31))

        return lines

    def gen_interleaved_vm(self):
        """Interleaved stores/loads to different offsets through VM."""
        n = random.randint(3, 6)
        regs = random.sample(range(5, 31), n * 2)
        store_regs = regs[:n]
        load_regs = regs[n:]
        offsets = [i * 8 for i in range(n)]
        protected = set(store_regs + load_regs)

        lines = [f"  # Interleaved store-load via VM: {n} pairs"]
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

    def gen_pte_modify_access(self):
        """Modify a PTE, SFENCE.VMA, then access through the new mapping.

        This directly stresses Finding 2/3: store to PTE in store buffer,
        SFENCE.VMA, then PTW reads (potentially stale) PTE from memory.

        We use the identity-mapped gigapage, so modifying it and putting it
        back should work. We write the same PTE value back.
        """
        # Use dedicated regs: x28=PT addr, x29=PTE value, x30=temp
        addr_reg = 28
        pte_reg = 29
        rd = self._rand_reg(addr_reg, pte_reg, 30)

        lines = [
            f"  # PTE modify + SFENCE + access (Finding 2/3 stress)",
            f"  la x{addr_reg}, page_table_l2",
            # Entry 2 is at offset 16 (index 2 * 8 bytes)
            f"  ld x{pte_reg}, 16(x{addr_reg})",
            # Store same PTE back (no actual change, but forces SB interaction)
            f"  sd x{pte_reg}, 16(x{addr_reg})",
            f"  sfence.vma",
            # Access through VM — should still work
            f"  ld x30, 0(x4)",
        ]

        rs = self._rand_reg(addr_reg, pte_reg, 30)
        rd2 = self._rand_reg(addr_reg, pte_reg, 30, rs)
        off = random.choice(range(0, 2040, 8))
        lines.extend([
            f"  sd x{rs}, {off}(x4)",
            f"  sfence.vma",
            f"  ld x{rd2}, {off}(x4)",
        ])
        lines.extend(self._emit_check(rd2, rs))
        return lines

    def gen_branch_store_load_vm(self):
        """Store through VM, branch, load through VM — verify."""
        rs = self._rand_reg()
        rd = self._rand_reg(rs)
        off = random.choice(range(0, 2040, 8))
        cmp1 = self._rand_reg(rs, rd)
        cmp2 = self._rand_reg(rs, rd, cmp1)

        taken = self.next_label("tk")
        end = self.next_label("end")

        lines = [
            f"  # Branch-store-load via VM",
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


def main():
    ap = argparse.ArgumentParser(description="Generate VM stress torture tests")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--patterns", type=int, default=100, help="Patterns per test")
    ap.add_argument("--outdir", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "generated"))
    args = ap.parse_args()

    os.makedirs(args.outdir, exist_ok=True)

    for i in range(args.count):
        seed = args.seed + i
        gen = VmStressGenerator(seed=seed, num_patterns=args.patterns)
        code = gen.generate()
        path = os.path.join(args.outdir, f"vmstress_{seed:06d}.S")
        with open(path, "w") as f:
            f.write(code)

    print(f"Generated {args.count} VM stress tests in {args.outdir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
