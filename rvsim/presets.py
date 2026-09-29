"""
Built-in configuration presets.

- ``basic`` — modest 4-wide OoO core, small caches, good for quick runs.
- ``fast``  — Apple M4 P-core class: 8-wide OoO at 4.4 GHz, 630-entry ROB,
  192KB L1I, 128KB L1D, 4MB L2, 36MB L3, the 64KB TAGE-SC-L with ITTAGE,
  4 unified FP/SIMD pipes, and DRAM controller.
- ``linux`` — a multi-core ``fast`` system with the memory map the
  bundled Linux image expects, kept coherent over an interconnect, with
  DDR5 memory.
- ``cortex_a72``, ``m1``, ``p550`` — models of the Arm Cortex-A72, an
  Apple M1-class core and the SiFive P550, from their published
  microarchitecture.

Usage from the CLI::

    rvsim mandelbrot.elf --preset fast

Usage from Python::

    from rvsim import presets, Simulator
    cfg = presets.fast()
    Simulator(cfg, binary="mandelbrot.elf").run()
"""

from typing import Optional

from .config import Config
from .config import (
    Backend,
    BranchPredictor,
    Cache,
    Coherence,
    Fu,
    HomeAgent,
    Interconnect,
    MemDepPredictor,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
)

__all__ = ["basic", "fast", "linux", "cortex_a72", "m1", "p550", "PRESETS"]

#: The device tree's ``timebase-frequency``, in MHz.
LINUX_TIMEBASE_MHZ = 10

INTERCONNECTS = {
    "crossbar": Interconnect.Crossbar,
    "ring": Interconnect.Ring,
    "mesh": Interconnect.Mesh,
    "torus": Interconnect.Torus,
    "hypercube": Interconnect.Hypercube,
}


def basic() -> Config:
    """Modest 4-wide out-of-order core with small caches.

    This is identical to ``Config()`` with no arguments.
    """
    return Config()


def fast() -> Config:
    """Apple M4 P-core class configuration.

    Based on publicly known M4 Everest P-core microarchitecture:

    - 8-wide rename/dispatch (Apple decodes up to ~10 but dispatches 8)
    - 630-entry ROB, 108-entry store buffer, ~160-entry issue queues
    - 6 integer pipes (4 simple ALU + 2 complex with mul/div)
    - 4 unified FP/SIMD pipes (each handles add, mul, FMA)
    - 2 branch units, 4 load/store AGUs (3 load + 2 store capable)
    - 192KB 6-way L1I, 128KB 8-way L1D (3-cycle hit), 4MB L2, 36MB L3
    - 4.4 GHz P-core clock
    - Seznec's 64KB TAGE-SC-L (CBP-5) with ITTAGE (Apple's predictor is
      proprietary but believed to be TAGE-class)
    - Non-inclusive cache hierarchy
    - LPDDR5-class DRAM controller

    Vector units are modeled as a RISC-V V equivalent of Apple's 4
    NEON/AMX pipes — VLEN=256 with 4 lanes and chaining.

    Note: the sim models FU types independently, so having count=4 for
    FpAdd/FpMul/FpFma slightly overstates mixed-FP throughput vs the
    real M4 (which has 4 *unified* pipes). The 8-wide dispatch width
    naturally limits total throughput to realistic levels.
    """
    return Config(
        # ── Frontend ─────────────────────────────────────────────────────
        width=8,
        cpu_clock_mhz=4400,
        mem_dep_predictor=MemDepPredictor.StoreSet(
            ssit_size=4096,
            lfst_size=512,
        ),
        branch_predictor=BranchPredictor.ScLTage(
            loop_log_size=10,
            ittage_num_banks=8,
            ittage_table_size=512,
            ittage_history_lengths=[4, 8, 16, 32, 64, 128, 256, 512],
            ittage_tag_widths=[9, 9, 10, 10, 11, 11, 12, 12],
            ittage_reset_interval=500_000,
        ),
        btb_size=16384,
        btb_ways=8,
        ras_size=48,
        # ── Out-of-order backend ─────────────────────────────────────────
        backend=Backend.OutOfOrder(
            rob_size=630,
            store_buffer_size=108,
            issue_queue_size=160,
            load_queue_size=140,
            load_ports=3,
            store_ports=2,
            prf_gpr_size=384,
            prf_fpr_size=256,
            prf_vpr_size=96,
            vec_chaining=True,
            fu_config=Fu(
                [
                    # 6 integer pipes (4 simple + 2 complex)
                    Fu.IntAlu(count=6, latency=1),
                    Fu.IntMul(count=2, latency=3),
                    Fu.IntDiv(count=1, latency=10),
                    # 4 unified FP/SIMD pipes
                    Fu.FpAdd(count=4, latency=3),
                    Fu.FpMul(count=4, latency=3),
                    Fu.FpFma(count=4, latency=4),
                    Fu.FpDivSqrt(count=1, latency=12),
                    # Control
                    Fu.Branch(count=2, latency=1),
                    # Memory (3 load + 2 store capable AGUs)
                    Fu.Mem(count=4, latency=1),
                    # Vector (RVV equivalent of 4 NEON/AMX pipes)
                    Fu.VecIntAlu(count=4, latency=1),
                    Fu.VecIntMul(count=2, latency=3),
                    Fu.VecIntDiv(count=1, latency=10),
                    Fu.VecFpAlu(count=4, latency=3),
                    Fu.VecFpFma(count=4, latency=4),
                    Fu.VecFpDivSqrt(count=1, latency=12),
                    Fu.VecMem(count=2, latency=1),
                    Fu.VecPermute(count=2, latency=2),
                ]
            ),
            checkpoint_count=64,
        ),
        # ── Vector ISA ───────────────────────────────────────────────────
        vlen=256,
        num_vec_lanes=4,
        # ── Cache hierarchy ──────────────────────────────────────────────
        l1i=Cache(
            size="192KB",
            line="64B",
            ways=6,
            policy=ReplacementPolicy.PLRU(),
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=3),
            mshr_count=10,
        ),
        l1d=Cache(
            size="128KB",
            line="64B",
            ways=8,
            policy=ReplacementPolicy.PLRU(),
            latency=3,
            prefetcher=Prefetcher.Stride(degree=4, table_size=512),
            mshr_count=20,
        ),
        l2=Cache(
            size="4MB",
            line="64B",
            ways=16,
            policy=ReplacementPolicy.PLRU(),
            latency=12,
            prefetcher=Prefetcher.Stream(degree=4),
            mshr_count=32,
        ),
        l3=Cache(
            size="36MB",
            line="64B",
            ways=16,
            policy=ReplacementPolicy.PLRU(),
            latency=35,
            prefetcher=Prefetcher.Tagged(degree=2),
            mshr_count=64,
        ),
        inclusion_policy=Cache.NINE(),
        wcb_entries=16,
        # ── Memory ───────────────────────────────────────────────────────
        tlb_size=160,
        l2_tlb_size=4096,
        l2_tlb_ways=8,
        l2_tlb_latency=4,
        memory_controller=MemoryController.DRAM(
            t_cas=12,
            t_ras=12,
            t_pre=12,
            row_miss_latency=90,
        ),
        bus_width=8,
        bus_latency=1,
    )


def linux(
    harts: int = 8,
    *,
    memory: str = "ddr5",
    speed_bin: str = "5600B",
    interconnect: str = "mesh",
    real_time: bool = True,
    core: Optional[Config] = None,
) -> Config:
    """``core`` (the ``fast`` preset by default) in a system that boots the
    bundled Linux image: its memory map, harts, coherence and memory
    replace the core config's.

    ``harts`` harts boot through OpenSBI's HSM into an SMP kernel; with
    more than one, the private caches are kept coherent by a snoop-filter
    home agent over ``interconnect`` (``crossbar``, ``ring``, ``mesh``,
    ``torus`` or ``hypercube``). ``memory`` is ``ddr5`` (JEDEC
    command-level timing at ``speed_bin``, four channels) or ``dram`` (the
    core config's own memory controller).

    With ``real_time`` the CLINT ticks at the device tree's 10 MHz
    timebase, so the guest's clock keeps time with the modelled one.
    Without it the CLINT ticks every cycle: guest time runs
    ``cpu_clock_mhz / 10`` times fast, which shortens a boot's sleeps and
    timeouts but floods measurements with timer interrupts.
    """
    base = core if core is not None else fast()
    if interconnect not in INTERCONNECTS:
        raise ValueError(
            f"interconnect must be one of {sorted(INTERCONNECTS)}, got {interconnect!r}"
        )
    if memory == "ddr5":
        controller = MemoryController.DDR5(speed_bin=speed_bin, channels=4)
    elif memory == "dram":
        controller = base.memory_controller
    else:
        raise ValueError(f"memory must be 'ddr5' or 'dram', got {memory!r}")
    return base.replace(
        ram_size=256 * 1024 * 1024,
        ram_base=0x80000000,
        uart_base=0x10000000,
        disk_base=0x10001000,
        clint_base=0x02000000,
        syscon_base=0x00100000,
        kernel_offset=0x200000,
        clint_divider=base.cpu_clock_mhz // LINUX_TIMEBASE_MHZ if real_time else 1,
        hart_count=harts,
        coherence=Coherence(
            home_agent=HomeAgent.SnoopFilter(),
            interconnect=INTERCONNECTS[interconnect](),
        ),
        memory_controller=controller,
    )


# Registry for CLI --preset lookup.
def cortex_a72():
    """Cortex-A72: 3-wide O3, 48KB I$, 32KB D$ (8 MSHRs), 1MB L2.

    ARM Cortex-A72 machine config.
    https://en.wikipedia.org/wiki/ARM_Cortex-A72

    Microarchitecture (publicly documented):
    - 3-wide fetch/decode/rename/dispatch/issue
    - Out-of-order execution, 128-entry ROB
    - 60-entry unified issue queue
    - 12-entry store buffer, 16-entry load queue
    - 2 load ports, 1 store port
    - PRF: 128 integer + 128 FP physical registers
    - Execution units:
        - 3x integer ALU (latency 1, includes shift/compare)
        - 1x integer multiplier (latency 3, pipelined)
        - 1x integer divider (latency ~20-39, non-pipelined)
        - 2x FP/NEON pipeline (modeled as FpAdd + FpMul/FpFma per pipe)
        - 1x FP div/sqrt (latency ~17-38, non-pipelined)
        - 1x branch unit (latency 1)
        - 2x load/store AGU (modeled as Mem units)
    - 48KB L1-I (3-way), 32KB L1-D (2-way), 8 MSHRs on L1-D
    - 1MB L2 (16-way unified), shared
    - TAGE-like branch predictor, 4096-entry BTB, 16-entry RAS
    """
    return Config(
        width=3,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=4,
            table_size=2048,
            reset_interval=1000,
            history_lengths=[8, 20, 50, 110],
            tag_widths=[8, 8, 9, 9],
        ),
        backend=Backend.OutOfOrder(
            rob_size=128,
            issue_queue_size=60,
            store_buffer_size=12,
            load_queue_size=16,
            load_ports=2,
            store_ports=1,
            prf_gpr_size=128,
            prf_fpr_size=128,
            fu_config=Fu(
                [
                    Fu.IntAlu(count=3, latency=1),
                    Fu.IntMul(count=1, latency=3),
                    Fu.IntDiv(count=1, latency=28),
                    Fu.FpAdd(count=2, latency=5),
                    Fu.FpMul(count=2, latency=5),
                    Fu.FpFma(count=2, latency=5),
                    Fu.FpDivSqrt(count=1, latency=17),
                    Fu.Branch(count=1, latency=1),
                    Fu.Mem(count=2, latency=1),
                ]
            ),
        ),
        btb_size=4096,
        ras_size=16,
        ram_size="256MB",
        tlb_size=48,
        l1i=Cache(
            size="48KB",
            line="64B",
            ways=3,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=2),
        ),
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=2,
            latency=1,
            mshr_count=8,
            prefetcher=Prefetcher.Stride(degree=1, table_size=32),
        ),
        l2=Cache(
            size="1MB",
            line="64B",
            ways=16,
            latency=12,
            mshr_count=16,
        ),
    )


def m1(
    *,
    branch_predictor=None,
    ram_size_bytes=0x1000_0000,
    pipeline_width=4,
):
    """M1-style: 4-wide, 128KB L1-I/D, 4MB L2."""
    if branch_predictor is None:
        branch_predictor = BranchPredictor.TAGE(
            num_banks=4,
            table_size=4096,
            reset_interval=2000,
            history_lengths=[5, 15, 44, 130],
            tag_widths=[9, 9, 10, 10],
        )

    return Config(
        width=pipeline_width,
        branch_predictor=branch_predictor,
        btb_size=8192,
        ras_size=64,
        initial_sp=0x8010_0000,
        ram_size=ram_size_bytes,
        l1i=Cache(
            size="128KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=2),
        ),
        l1d=Cache(
            size="128KB",
            line="64B",
            ways=8,
            latency=1,
            mshr_count=12,
            prefetcher=Prefetcher.Stride(degree=2, table_size=128),
        ),
        l2=Cache(
            size="4MB",
            line="64B",
            ways=16,
            latency=12,
            mshr_count=32,
        ),
    )


def p550(
    *,
    branch_predictor=None,
    ram_size_bytes="256MB",
    pipeline_width=3,
):
    """SiFive Performance P550 — 3-wide, 13-stage, out-of-order.

    Microarchitecture notes (from Chips and Cheese reverse engineering):
    - 3-wide fetch/decode/rename/retire
    - ROB ~72 entries (comparable to Core 2 / Goldmont Plus class)
    - Modest issue queue, ~32 entries estimated
    - Load queue ~24 entries, store buffer ~16 entries (described as "thin")
    - PRF sized with "plenty of capacity compared to ROB size"
    - 9.1 KiB branch history table with good pattern recognition
    - 32-entry BTB handles taken branches with zero bubbles
    - 32KB 8-way L1i, 32KB 8-way L1d, private L2 per core
    - 4 MB shared L3 on EIC7700X implementation
    - 13-stage pipeline → ~11-13 cycle mispredict penalty
    - No hardware misaligned access support (trap-based emulation)

        SiFive Performance P550 machine config.

        Based on published microarchitecture analysis:
        - Chips and Cheese: "Inside SiFive's P550 Microarchitecture" (Jan 2025)
        - SiFive official specs: 13-stage, triple-issue, out-of-order, RV64GC
        - Measured on Eswin EIC7700X SoC @ 1.4 GHz, 32KB+32KB L1, private L2, 4MB shared L3
        - Published SPECInt2006: 8.65/GHz
        - Observed IPC: approaching 3.0 on favorable workloads
    """
    if branch_predictor is None:
        # P550 has a 9.1 KiB BHT — predictor type unconfirmed.
        # Tournament sizing to match ~9 KB budget:
        #   global predictor:  2^13 entries × 2b = 2 KB
        #   choice (selector): 2^13 entries × 2b = 2 KB
        #   local hist table:  2^11 entries × 11b ≈ 2.75 KB
        #   local predictor:   2^11 entries × 2b  = 0.5 KB
        #   total ≈ 7.25 KB (closest we can get within the budget)
        branch_predictor = BranchPredictor.Tournament(
            global_size_bits=13,
            local_hist_bits=11,
            local_pred_bits=11,
        )

    return Config(
        width=pipeline_width,
        backend=Backend.OutOfOrder(
            rob_size=72,  # Modest, Core 2 / Goldmont Plus class
            issue_queue_size=32,  # "more scheduling capacity" than A75
            load_queue_size=24,  # "memory ordering queues can be a bit thin"
            store_buffer_size=16,  # Conservative estimate from "thin" description
            prf_gpr_size=128,  # "plenty of register file capacity compared to ROB"
            prf_fpr_size=96,  # Proportional to GPR, RV64GC needs FP regs
            load_ports=1,
            store_ports=1,
            fu_config=Fu(
                [
                    # "more flexible integer port setup" than A75
                    # P550 has 3 integer ALU ports based on execution width analysis
                    Fu.IntAlu(count=3, latency=1),
                    Fu.IntMul(count=1, latency=3),
                    Fu.IntDiv(count=1, latency=12),
                    # FP — single pipeline handles add/mul/fma; model as one
                    # of each since rvsim uses separate type classes.
                    # All share the same 5-cycle latency (P550 FP pipeline).
                    Fu.FpAdd(count=1, latency=5),
                    Fu.FpMul(count=1, latency=5),
                    Fu.FpFma(count=1, latency=5),
                    Fu.FpDivSqrt(count=1, latency=15),
                    Fu.Branch(count=1, latency=1),
                    Fu.Mem(count=1, latency=1),
                ]
            ),
        ),
        branch_predictor=branch_predictor,  # type: ignore[arg-type]
        btb_size=32,  # 32-entry BTB, zero-bubble taken branches
        ras_size=16,  # Modest RAS for low-power core
        initial_sp=0x8010_0000,
        ram_size=ram_size_bytes,
        # L1i: 32KB, 8-way, 64B lines — confirmed by SiFive specs
        l1i=Cache(
            size="32KB",
            line="64B",
            ways=8,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=1),
        ),
        # L1d: 32KB, 8-way, 64B lines — confirmed by SiFive specs
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=8,
            latency=3,  # 3-cycle load-to-use (typical for this class)
            mshr_count=8,  # Non-blocking, modest MSHR count
            prefetcher=Prefetcher.Stride(degree=1, table_size=64),
        ),
        # Private L2 per core — size not publicly confirmed,
        # 256KB is consistent with area-optimized OoO cores
        l2=Cache(
            size="256KB",
            line="64B",
            ways=8,
            latency=10,
            mshr_count=16,
        ),
        # 4 MB shared L3 on EIC7700X — modeling single-core view
        l3=Cache(
            size="4MB",
            line="64B",
            ways=16,
            latency=30,
            mshr_count=32,
        ),
        # LPDDR5-6400 on the Premier P550 dev board
        memory_controller=MemoryController.DRAM(
            t_cas=14,
            t_ras=14,
            row_miss_latency=120,
        ),
    )


PRESETS = {
    "basic": basic,
    "fast": fast,
    "cortex_a72": cortex_a72,
    "m1": m1,
    "p550": p550,
}
