#!/usr/bin/env python3
"""Generate random self-checking RISC-V torture tests for OoO pipeline verification.

Each generated test:
  1. Initializes registers with known values
  2. Runs a random sequence of ALU, memory, and branch instructions
  3. Stores final register state to a signature region
  4. Uses the riscv-tests HTIF pass/fail mechanism

The tests are designed to stress:
  - RAW / WAW / WAR data hazards across the rename map
  - Store-to-load forwarding and memory ordering
  - Branch misprediction recovery (speculative state cleanup)
  - Register pressure on the physical register file
  - Mixed integer/memory instruction scheduling

Usage:
    python generate.py [--seed SEED] [--count N] [--length L] [--outdir DIR]
"""

import argparse
import os
import random
import sys

# Registers available for torture (avoid x0/zero, x1/ra, x2/sp, x3/gp=TESTNUM, x4/tp)
# Use x5-x31 as the torture register pool
REGS = list(range(5, 32))
# Dedicated scratch registers for memory ops (not in the random pool)
MEM_BASE_REG = 4  # tp -- we'll set this to point at scratch memory
# We use x3 (gp) as TESTNUM per riscv-tests convention


def rand_reg():
    """Pick a random register from the pool."""
    return random.choice(REGS)


def rand_imm(bits=12):
    """Random sign-extended immediate."""
    return random.randint(-(1 << (bits - 1)), (1 << (bits - 1)) - 1)


def rand_small_imm():
    """Small immediate for shifts etc."""
    return random.randint(0, 63)


def rand_offset():
    """Random aligned memory offset (8-byte aligned, within ±2KB scratch)."""
    return random.choice(range(0, 2048, 8))


class TortureGenerator:
    def __init__(self, seed, length, mem_ops_pct=30, branch_pct=15):
        self.rng = random.Random(seed)
        self.seed = seed
        self.length = length
        self.mem_ops_pct = mem_ops_pct
        self.branch_pct = branch_pct
        random.seed(seed)

    def gen_alu_inst(self):
        """Generate a random ALU instruction."""
        rd = rand_reg()
        rs1 = rand_reg()
        rs2 = rand_reg()
        imm = rand_imm()
        shimm = rand_small_imm()

        ops = [
            # R-type
            f"add x{rd}, x{rs1}, x{rs2}",
            f"sub x{rd}, x{rs1}, x{rs2}",
            f"and x{rd}, x{rs1}, x{rs2}",
            f"or  x{rd}, x{rs1}, x{rs2}",
            f"xor x{rd}, x{rs1}, x{rs2}",
            f"sll x{rd}, x{rs1}, x{rs2}",
            f"srl x{rd}, x{rs1}, x{rs2}",
            f"sra x{rd}, x{rs1}, x{rs2}",
            f"slt x{rd}, x{rs1}, x{rs2}",
            f"sltu x{rd}, x{rs1}, x{rs2}",
            # RV64 W-variants
            f"addw x{rd}, x{rs1}, x{rs2}",
            f"subw x{rd}, x{rs1}, x{rs2}",
            f"sllw x{rd}, x{rs1}, x{rs2}",
            f"srlw x{rd}, x{rs1}, x{rs2}",
            f"sraw x{rd}, x{rs1}, x{rs2}",
            # I-type
            f"addi x{rd}, x{rs1}, {imm}",
            f"andi x{rd}, x{rs1}, {imm}",
            f"ori  x{rd}, x{rs1}, {imm}",
            f"xori x{rd}, x{rs1}, {imm}",
            f"slti x{rd}, x{rs1}, {imm}",
            f"sltiu x{rd}, x{rs1}, {imm}",
            f"slli x{rd}, x{rs1}, {shimm}",
            f"srli x{rd}, x{rs1}, {shimm}",
            f"srai x{rd}, x{rs1}, {shimm}",
            # RV64I W-variants
            f"addiw x{rd}, x{rs1}, {imm}",
            f"slliw x{rd}, x{rs1}, {shimm & 0x1f}",
            f"srliw x{rd}, x{rs1}, {shimm & 0x1f}",
            f"sraiw x{rd}, x{rs1}, {shimm & 0x1f}",
            # LUI / AUIPC
            f"lui x{rd}, {imm & 0xFFFFF}",
        ]

        # M extension (multiply/divide)
        m_ops = [
            f"mul x{rd}, x{rs1}, x{rs2}",
            f"mulh x{rd}, x{rs1}, x{rs2}",
            f"mulhu x{rd}, x{rs1}, x{rs2}",
            f"mulhsu x{rd}, x{rs1}, x{rs2}",
            f"mulw x{rd}, x{rs1}, x{rs2}",
        ]
        # Include div ops with lower probability (they're slow)
        div_ops = [
            f"div x{rd}, x{rs1}, x{rs2}",
            f"divu x{rd}, x{rs1}, x{rs2}",
            f"rem x{rd}, x{rs1}, x{rs2}",
            f"remu x{rd}, x{rs1}, x{rs2}",
            f"divw x{rd}, x{rs1}, x{rs2}",
            f"divuw x{rd}, x{rs1}, x{rs2}",
            f"remw x{rd}, x{rs1}, x{rs2}",
            f"remuw x{rd}, x{rs1}, x{rs2}",
        ]

        all_ops = ops + m_ops
        if random.random() < 0.2:
            all_ops += div_ops

        return random.choice(all_ops)

    def gen_mem_inst(self):
        """Generate a load or store using tp as base register."""
        rd = rand_reg()
        rs = rand_reg()
        off = rand_offset()

        loads = [
            f"ld x{rd}, {off}(x{MEM_BASE_REG})",
            f"lw x{rd}, {off}(x{MEM_BASE_REG})",
            f"lwu x{rd}, {off}(x{MEM_BASE_REG})",
            f"lh x{rd}, {off & ~1}(x{MEM_BASE_REG})",
            f"lhu x{rd}, {off & ~1}(x{MEM_BASE_REG})",
            f"lb x{rd}, {off}(x{MEM_BASE_REG})",
            f"lbu x{rd}, {off}(x{MEM_BASE_REG})",
        ]
        stores = [
            f"sd x{rs}, {off}(x{MEM_BASE_REG})",
            f"sw x{rs}, {off}(x{MEM_BASE_REG})",
            f"sh x{rs}, {off & ~1}(x{MEM_BASE_REG})",
            f"sb x{rs}, {off}(x{MEM_BASE_REG})",
        ]

        if random.random() < 0.6:
            return random.choice(loads)
        else:
            return random.choice(stores)

    def gen_mem_hazard_sequence(self):
        """Generate a store-load forwarding hazard sequence."""
        rs = rand_reg()
        rd = rand_reg()
        off = rand_offset()

        lines = []
        lines.append(f"  sd x{rs}, {off}(x{MEM_BASE_REG})")
        # Insert 0-3 ALU ops between store and load
        for _ in range(random.randint(0, 3)):
            lines.append(f"  {self.gen_alu_inst()}")
        lines.append(f"  ld x{rd}, {off}(x{MEM_BASE_REG})")
        return lines

    def gen_width_mismatch_forwarding(self):
        """Store with one width, load with a different width from the same or overlapping address.

        This catches store-buffer forwarding bugs where partial forwarding
        is incorrect (e.g., sw then ld, or sd then lw from offset+4).
        """
        rs = rand_reg()
        rd = rand_reg()
        off = random.choice(range(0, 2040, 8))

        patterns = [
            # Store wide, load narrow (must forward subset of bytes)
            (f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lw x{rd}, {off}(x{MEM_BASE_REG})"),
            (f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lwu x{rd}, {off}(x{MEM_BASE_REG})"),
            (f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lw x{rd}, {off + 4}(x{MEM_BASE_REG})"),
            (f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lh x{rd}, {off + 2}(x{MEM_BASE_REG})"),
            (f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lb x{rd}, {off + 3}(x{MEM_BASE_REG})"),
            # Store narrow, load wide (cannot fully forward -- must go to cache)
            (f"  sw x{rs}, {off}(x{MEM_BASE_REG})",
             f"  ld x{rd}, {off}(x{MEM_BASE_REG})"),
            (f"  sh x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lw x{rd}, {off}(x{MEM_BASE_REG})"),
            (f"  sb x{rs}, {off}(x{MEM_BASE_REG})",
             f"  ld x{rd}, {off}(x{MEM_BASE_REG})"),
            # Store narrow, load narrow at same offset
            (f"  sw x{rs}, {off}(x{MEM_BASE_REG})",
             f"  lw x{rd}, {off}(x{MEM_BASE_REG})"),
            (f"  sh x{rs}, {off & ~1}(x{MEM_BASE_REG})",
             f"  lh x{rd}, {off & ~1}(x{MEM_BASE_REG})"),
        ]

        store_inst, load_inst = random.choice(patterns)
        lines = [store_inst]
        # Insert 0-2 ALU ops between store and load
        for _ in range(random.randint(0, 2)):
            lines.append(f"  {self.gen_alu_inst()}")
        lines.append(load_inst)
        return lines

    def gen_overlapping_stores(self):
        """Multiple stores to overlapping addresses, then a load.

        Tests that the store buffer correctly merges or orders overlapping writes.
        """
        rs1 = rand_reg()
        rs2 = rand_reg()
        rd = rand_reg()
        off = random.choice(range(0, 2040, 8))

        patterns = [
            # Two stores to same address, different widths
            [f"  sd x{rs1}, {off}(x{MEM_BASE_REG})",
             f"  sw x{rs2}, {off}(x{MEM_BASE_REG})",
             f"  ld x{rd}, {off}(x{MEM_BASE_REG})"],
            # Store doubleword, then overwrite upper half
            [f"  sd x{rs1}, {off}(x{MEM_BASE_REG})",
             f"  sw x{rs2}, {off + 4}(x{MEM_BASE_REG})",
             f"  ld x{rd}, {off}(x{MEM_BASE_REG})"],
            # Two byte stores then word load
            [f"  sb x{rs1}, {off}(x{MEM_BASE_REG})",
             f"  sb x{rs2}, {off + 1}(x{MEM_BASE_REG})",
             f"  lh x{rd}, {off}(x{MEM_BASE_REG})"],
            # Three stores narrowing, then wide load
            [f"  sd x{rs1}, {off}(x{MEM_BASE_REG})",
             f"  sw x{rs2}, {off}(x{MEM_BASE_REG})",
             f"  sh x{rs1}, {off}(x{MEM_BASE_REG})",
             f"  ld x{rd}, {off}(x{MEM_BASE_REG})"],
        ]

        lines = random.choice(patterns)
        return [l for l in lines]  # return a copy

    def gen_store_buffer_stress(self):
        """Fill the store buffer with many stores, then load from various addresses.

        Stresses store buffer capacity and forwarding under pressure.
        """
        lines = []
        n_stores = random.randint(4, 12)
        offsets = [random.choice(range(0, 2048, 8)) for _ in range(n_stores)]

        # Emit stores
        for off in offsets:
            rs = rand_reg()
            width = random.choice(["sd", "sw", "sh", "sb"])
            if width in ("sh",):
                off = off & ~1
            lines.append(f"  {width} x{rs}, {off}(x{MEM_BASE_REG})")

        # Emit loads from some of the same offsets
        for off in random.sample(offsets, min(len(offsets), random.randint(2, 6))):
            rd = rand_reg()
            load_width = random.choice(["ld", "lw", "lwu", "lh", "lhu", "lb", "lbu"])
            if load_width in ("lh", "lhu"):
                off = off & ~1
            lines.append(f"  {load_width} x{rd}, {off}(x{MEM_BASE_REG})")

        return [f"  {l.strip()}" if not l.startswith("  ") else l for l in lines]

    def gen_speculative_mem_sequence(self, label_id):
        """Memory operations on a mispredicted branch path.

        Tests that speculative stores are properly squashed on branch misprediction
        and that speculative loads don't corrupt architectural state.
        """
        rs1 = rand_reg()
        rs2 = rand_reg()
        rd = rand_reg()
        off1 = random.choice(range(0, 2048, 8))
        off2 = random.choice(range(0, 2048, 8))

        bop = random.choice(["beq", "bne", "blt", "bge"])
        taken = f".Lspec_taken_{self.seed}_{label_id}"
        end = f".Lspec_end_{self.seed}_{label_id}"

        lines = [
            f"  # Speculative memory test",
            f"  sd x{rs1}, {off1}(x{MEM_BASE_REG})",  # committed store
            f"  {bop} x{rs1}, x{rs2}, {taken}",
            # Not-taken path: stores that may or may not execute
            f"  sd x{rs2}, {off1}(x{MEM_BASE_REG})",  # overwrites if not-taken
            f"  ld x{rd}, {off2}(x{MEM_BASE_REG})",
            f"  {self.gen_alu_inst()}",
            f"  j {end}",
            f"{taken}:",
            # Taken path: different stores
            f"  sd x{rs1}, {off2}(x{MEM_BASE_REG})",
            f"  ld x{rd}, {off1}(x{MEM_BASE_REG})",  # should see committed store value
            f"  {self.gen_alu_inst()}",
            f"{end}:",
            f"  ld x{rd}, {off1}(x{MEM_BASE_REG})",  # verify correct value visible
        ]
        return lines

    def gen_fence_mem_ordering(self):
        """Store, fence, load sequence to test fence correctness.

        The fence must ensure the store is visible before the load executes.
        """
        rs = rand_reg()
        rd = rand_reg()
        off = random.choice(range(0, 2048, 8))

        fence_types = [
            "fence",
            "fence rw, rw",
            "fence w, r",
            "fence w, w",
            "fence r, r",
            "fence.i",
        ]

        lines = [
            f"  sd x{rs}, {off}(x{MEM_BASE_REG})",
            f"  {random.choice(fence_types)}",
            f"  ld x{rd}, {off}(x{MEM_BASE_REG})",
        ]
        # Sometimes add ALU ops around the fence
        if random.random() < 0.5:
            lines.insert(1, f"  {self.gen_alu_inst()}")
        return lines

    def gen_amo_sequence(self):
        """Generate atomic memory operation sequences.

        Tests AMO instruction correctness and interaction with store buffer.
        """
        rd = rand_reg()
        rs = rand_reg()
        rt = rand_reg()
        off = random.choice(range(0, 2040, 8))

        # We need the address in a register for AMO (rs1)
        # Use a temporary to compute address
        addr_reg = rand_reg()
        while addr_reg in (rd, rs):
            addr_reg = rand_reg()

        amo_ops_64 = [
            f"  amoadd.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amoswap.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amoand.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amoor.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amoxor.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amomin.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amomax.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amominu.d x{rd}, x{rs}, (x{addr_reg})",
            f"  amomaxu.d x{rd}, x{rs}, (x{addr_reg})",
        ]
        amo_ops_32 = [
            f"  amoadd.w x{rd}, x{rs}, (x{addr_reg})",
            f"  amoswap.w x{rd}, x{rs}, (x{addr_reg})",
            f"  amoand.w x{rd}, x{rs}, (x{addr_reg})",
            f"  amoor.w x{rd}, x{rs}, (x{addr_reg})",
            f"  amoxor.w x{rd}, x{rs}, (x{addr_reg})",
        ]

        lines = [
            f"  addi x{addr_reg}, x{MEM_BASE_REG}, {off}",
            f"  sd x{rs}, 0(x{addr_reg})",  # init memory
        ]

        if random.random() < 0.6:
            lines.append(random.choice(amo_ops_64))
        else:
            lines.append(random.choice(amo_ops_32))

        # Load back the result to check
        lines.append(f"  ld x{rt}, 0(x{addr_reg})")
        return lines

    def gen_lr_sc_sequence(self):
        """Generate LR/SC (load-reserved/store-conditional) sequences."""
        rd = rand_reg()
        rs = rand_reg()
        rt = rand_reg()
        off = random.choice(range(0, 2040, 8))
        addr_reg = rand_reg()
        while addr_reg in (rd, rs, rt):
            addr_reg = rand_reg()

        if random.random() < 0.5:
            # 64-bit LR/SC
            lines = [
                f"  addi x{addr_reg}, x{MEM_BASE_REG}, {off}",
                f"  lr.d x{rd}, (x{addr_reg})",
                f"  sc.d x{rt}, x{rs}, (x{addr_reg})",
            ]
        else:
            # 32-bit LR/SC
            lines = [
                f"  addi x{addr_reg}, x{MEM_BASE_REG}, {off}",
                f"  lr.w x{rd}, (x{addr_reg})",
                f"  sc.w x{rt}, x{rs}, (x{addr_reg})",
            ]

        return lines

    def gen_pointer_chase(self):
        """Store an address to memory, load it back, and use it to access memory.

        Mimics linked list traversal patterns that caused the Linux crash.
        """
        rd = rand_reg()
        rs = rand_reg()
        off1 = random.choice(range(0, 2040, 8))
        off2 = random.choice(range(0, 2040, 8))

        lines = [
            f"  # Pointer chase pattern",
            f"  addi x{rd}, x{MEM_BASE_REG}, {off2}",  # create a valid pointer
            f"  sd x{rd}, {off1}(x{MEM_BASE_REG})",     # store pointer to memory
        ]
        # Optional ALU noise
        for _ in range(random.randint(0, 2)):
            lines.append(f"  {self.gen_alu_inst()}")
        # Load the pointer back
        lines.append(f"  ld x{rd}, {off1}(x{MEM_BASE_REG})")
        # Dereference it (read from the pointed-to location)
        lines.append(f"  ld x{rs}, 0(x{rd})")
        return lines

    def gen_raw_hazard_sequence(self):
        """Generate a read-after-write hazard chain."""
        r1 = rand_reg()
        r2 = rand_reg()
        r3 = rand_reg()
        imm1 = rand_imm()

        lines = [
            f"  addi x{r1}, x{r2}, {imm1}",
            f"  add x{r3}, x{r1}, x{r2}",  # RAW on r1
            f"  xor x{r2}, x{r3}, x{r1}",  # RAW on r3
        ]
        return lines

    def gen_waw_hazard_sequence(self):
        """Generate a write-after-write hazard (same rd, different sources)."""
        rd = rand_reg()
        rs1 = rand_reg()
        rs2 = rand_reg()
        imm = rand_imm()

        lines = [
            f"  addi x{rd}, x{rs1}, {imm}",
            f"  add x{rd}, x{rs1}, x{rs2}",  # WAW: same rd
        ]
        return lines

    def gen_branch_block(self, label_id):
        """Generate a branch with ALU ops on both taken and not-taken paths."""
        rs1 = rand_reg()
        rs2 = rand_reg()

        branch_ops = ["beq", "bne", "blt", "bge", "bltu", "bgeu"]
        bop = random.choice(branch_ops)

        taken_label = f".Ltaken_{self.seed}_{label_id}"
        end_label = f".Lend_{self.seed}_{label_id}"

        lines = [
            f"  {bop} x{rs1}, x{rs2}, {taken_label}",
        ]
        # Not-taken path
        for _ in range(random.randint(1, 4)):
            lines.append(f"  {self.gen_alu_inst()}")
        lines.append(f"  j {end_label}")
        lines.append(f"{taken_label}:")
        # Taken path
        for _ in range(random.randint(1, 4)):
            lines.append(f"  {self.gen_alu_inst()}")
        lines.append(f"{end_label}:")
        return lines

    def gen_fence_sequence(self):
        """Generate a fence instruction."""
        fences = [
            "  fence",
            "  fence rw, rw",
            "  fence w, r",
            "  fence r, w",
        ]
        return [random.choice(fences)]

    def generate(self):
        """Generate a complete torture test assembly file."""
        lines = []

        lines.append(f"# Torture test -- seed {self.seed}, length {self.length}")
        lines.append('#include "riscv_test.h"')
        lines.append('#include "test_macros.h"')
        lines.append("")
        lines.append("RVTEST_RV64U")
        lines.append("RVTEST_CODE_BEGIN")
        lines.append("")

        # Initialize all torture registers with distinct nonzero values
        lines.append("  # Initialize registers")
        rng_init = random.Random(self.seed + 0xDEAD)
        for r in REGS:
            val = rng_init.randint(1, (1 << 63) - 1)
            lines.append(f"  li x{r}, 0x{val:016x}")
        lines.append("")

        # Set up memory base (tp) to point at scratch area
        lines.append("  # Set up memory scratch base")
        lines.append("  la x4, scratch_data")
        lines.append("")

        # Pre-initialize scratch memory with known pattern
        lines.append("  # Initialize scratch memory")
        lines.append("  li x5, 0xCAFEBABEDEADBEEF")
        for off in range(0, 2048, 8):
            lines.append(f"  sd x5, {off}(x4)")
        lines.append("")

        # Re-initialize x5 to its proper torture value
        rng_init2 = random.Random(self.seed + 0xDEAD)
        for r in REGS:
            v = rng_init2.randint(1, (1 << 63) - 1)
            if r == 5:
                lines.append(f"  li x5, 0x{v:016x}")
                break
        lines.append("")

        # Generate the torture body
        label_id = 0
        spec_label_id = 0
        i = 0
        while i < self.length:
            roll = random.random() * 100

            if roll < 3:
                seq = self.gen_mem_hazard_sequence()
                lines.extend(seq)
                i += len(seq)
            elif roll < 8:
                seq = self.gen_width_mismatch_forwarding()
                lines.extend(seq)
                i += len(seq)
            elif roll < 12:
                seq = self.gen_overlapping_stores()
                lines.extend(seq)
                i += len(seq)
            elif roll < 15:
                seq = self.gen_store_buffer_stress()
                lines.extend(seq)
                i += len(seq)
            elif roll < 19:
                seq = self.gen_raw_hazard_sequence()
                lines.extend(seq)
                i += len(seq)
            elif roll < 22:
                seq = self.gen_waw_hazard_sequence()
                lines.extend(seq)
                i += len(seq)
            elif roll < 22 + self.branch_pct:
                if random.random() < 0.3:
                    seq = self.gen_speculative_mem_sequence(spec_label_id)
                    spec_label_id += 1
                else:
                    seq = self.gen_branch_block(label_id)
                    label_id += 1
                lines.extend(seq)
                i += len(seq) - 2  # labels don't count
            elif roll < 22 + self.branch_pct + self.mem_ops_pct:
                lines.append(f"  {self.gen_mem_inst()}")
                i += 1
            elif roll < 22 + self.branch_pct + self.mem_ops_pct + 2:
                seq = self.gen_fence_mem_ordering()
                lines.extend(seq)
                i += len(seq)
            elif roll < 22 + self.branch_pct + self.mem_ops_pct + 4:
                seq = self.gen_amo_sequence()
                lines.extend(seq)
                i += len(seq)
            elif roll < 22 + self.branch_pct + self.mem_ops_pct + 5:
                seq = self.gen_lr_sc_sequence()
                lines.extend(seq)
                i += len(seq)
            elif roll < 22 + self.branch_pct + self.mem_ops_pct + 7:
                seq = self.gen_pointer_chase()
                lines.extend(seq)
                i += len(seq)
            else:
                lines.append(f"  {self.gen_alu_inst()}")
                i += 1

        lines.append("")

        # Signature dump: store all register values to signature region
        lines.append("  # Dump register state to signature")
        lines.append("  la x4, begin_signature")
        off = 0
        for r in REGS:
            if r == 4:
                continue  # tp is our base, skip it
            lines.append(f"  sd x{r}, {off}(x4)")
            off += 8
        lines.append("")

        lines.append("  RVTEST_PASS")
        lines.append("")
        lines.append("RVTEST_CODE_END")
        lines.append("")

        # Data section
        lines.append("  .data")
        lines.append("RVTEST_DATA_BEGIN")
        lines.append("")
        lines.append("  .align 4")
        # begin_signature / end_signature are defined by the RVTEST macros,
        # but we need the scratch data area
        lines.append("")
        lines.append("  # Scratch memory for loads/stores (2KB, 8-byte aligned)")
        lines.append("  .align 4")
        lines.append("scratch_data:")
        lines.append("  .fill 2048, 1, 0")
        lines.append("")
        lines.append("RVTEST_DATA_END")

        return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description="Generate RISC-V torture tests")
    ap.add_argument("--seed", type=int, default=0, help="Starting seed")
    ap.add_argument("--count", type=int, default=100, help="Number of tests to generate")
    ap.add_argument("--length", type=int, default=500, help="Instructions per test")
    ap.add_argument(
        "--outdir",
        default=os.path.join(
            os.path.dirname(os.path.abspath(__file__)), "generated"
        ),
        help="Output directory",
    )
    ap.add_argument(
        "--mem-pct", type=int, default=30, help="Percentage of memory operations"
    )
    ap.add_argument(
        "--branch-pct", type=int, default=15, help="Percentage of branch blocks"
    )
    args = ap.parse_args()

    os.makedirs(args.outdir, exist_ok=True)

    for i in range(args.count):
        seed = args.seed + i
        gen = TortureGenerator(
            seed=seed,
            length=args.length,
            mem_ops_pct=args.mem_pct,
            branch_pct=args.branch_pct,
        )
        code = gen.generate()
        path = os.path.join(args.outdir, f"torture_{seed:06d}.S")
        with open(path, "w") as f:
            f.write(code)

    print(f"Generated {args.count} torture tests in {args.outdir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
