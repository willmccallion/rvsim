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
- ``rocket``, ``boom`` — Chipyard's Rocket and Medium BOOM as their RTL
  builds them, the reference cores of the RTL comparison
  (``docs/rtl-rocket.md``, ``docs/rtl-boom.md``).

Usage from the CLI::

    rvsim mandelbrot.elf --preset fast

Usage from Python::

    from rvsim import presets, Simulator
    cfg = presets.fast()
    Simulator(cfg, binary="mandelbrot.elf").run()
"""

from .config import (
    Backend,
    BranchPredictor,
    Cache,
    Coherence,
    Config,
    Fu,
    HomeAgent,
    Interconnect,
    LoadPrefetcher,
    MemDepPredictor,
    MemoryController,
    PageBoundary,
    Prefetcher,
    ReplacementPolicy,
    StorePrefetcher,
)

__all__ = [
    "PRESETS",
    "basic",
    "boom",
    "cortex_a72",
    "fast",
    "linux",
    "m1",
    "p550",
    "rocket",
]

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
    core: Config | None = None,
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


def cortex_a72():
    """Cortex-A72: 3-wide O3, 48KB I$, 32KB D$ (8 MSHRs), 1MB L2.

    ARM Cortex-A72 machine config.
    https://en.wikipedia.org/wiki/ARM_Cortex-A72

    Microarchitecture (publicly documented):
    - 3-wide fetch/decode/rename/dispatch/issue
    - Out-of-order execution, 128-entry ROB
    - Eight issue queues of 8 entries (10 for branches), 66 in all; modelled
      as one 66-entry queue until per-pipe queues exist
    - 16-entry store queue, 32-entry load queue
    - One load AGU, one store AGU
    - PRF: 128 integer + 128 FP physical registers
    - Execution units (Chips and Cheese, Graviton at 2.3 GHz):
        - 2x integer ALU (latency 1), 1x multi-cycle pipe (multiply 3,
          divide non-pipelined)
        - 2x FP/NEON pipeline (modeled as FpAdd + FpMul/FpFma per pipe)
        - 1x FP div/sqrt (latency ~17-38, non-pipelined)
        - 1x branch unit (latency 1)
    - 48KB L1-I (3-way), 32KB L1-D (2-way, 4-cycle load-to-use), 8 MSHRs
    - 1MB L2 (16-way) on the Raspberry Pi 4's BCM2711, 21 cycles
    - 48-entry L1 ITLB, 32-entry L1 DTLB, 1024-entry 4-way L2 TLB
    - 4096-entry BTB, 31-entry return stack; mispredict penalty ~15 cycles
    - Load/store prefetcher (TRM 6.4.9): loads prefetch into the L1D and
      22 requests ahead into the L2 (CPUECTLR_EL1 reset), crossing pages
      through the TLB (CPUACTLR_EL1[43] reset); store misses prefetch into
      the L2 only. The L1D distance, table sizes and store run length are
      not published.
    - Clocked at 1.5 GHz as on the Raspberry Pi 4
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
            issue_queue_size=66,
            store_buffer_size=16,
            load_queue_size=32,
            load_ports=1,
            store_ports=1,
            prf_gpr_size=128,
            prf_fpr_size=128,
            fu_config=Fu(
                [
                    Fu.IntAlu(count=2, latency=1),
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
        ras_size=31,
        cpu_clock_mhz=1500,  # Raspberry Pi 4
        ram_size="256MB",
        tlb_size=32,  # 32-entry L1 DTLB
        l2_tlb_size=1024,
        l2_tlb_ways=4,
        l1i=Cache(
            size="48KB",
            line="64B",
            ways=3,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=2),
        ),
        # A hit is one cycle of address generation plus `latency`.
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=2,
            latency=3,  # 4-cycle load-to-use (Chips and Cheese)
            mshr_count=8,
        ),
        load_prefetcher=LoadPrefetcher.Stride(
            table_size=32,
            l1_lines=1,
            l2_lines=22,
            page_boundary=PageBoundary.CrossWithTlb(),
        ),
        store_prefetcher=StorePrefetcher.Stream(streams=4, l2_lines=8),
        # 1MB 16-way L2 on the BCM2711; 21 cycles measured on the A72.
        l2=Cache(
            size="1MB",
            line="64B",
            ways=16,
            latency=20,
            mshr_count=16,
        ),
        # LPDDR4-3200 on the Raspberry Pi 4: tRCD, tRP and CL of about
        # 18 ns are 27 cycles each at 1.5 GHz; the rest of the measured
        # 162 ns (243 cycles) random-access latency is the fabric and
        # controller, carried by the bus crossing each way.
        memory_controller=MemoryController.DRAM(
            t_cas=27,
            t_ras=27,
            t_pre=27,
        ),
        bus_latency=67,
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
    - 32KB 4-way L1i (3-cycle), 32KB 4-way L1d (3-cycle load-to-use), 64B lines
    - 256KB 8-way private L2 at 13 cycles; 4 MB L3 at ~38 cycles on the EIC7700X
    - DRAM at 194 ns on the HiFive Premier P550 (272 cycles at 1.4 GHz)
    - FP add, multiply and FMA at 4 cycles; one load AGU and one store AGU
    - 32-entry fully associative L1 TLBs, 512-entry L2 TLB
    - 13-stage pipeline → ~11-13 cycle mispredict penalty
    - No hardware misaligned access support (trap-based emulation)
    - Prefetchers unpublished: a load stride prefetcher that keeps to the
      page, and no store prefetcher, as the cautious reading

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
                    # Chips and Cheese measured 4 cycles for all three.
                    Fu.FpAdd(count=1, latency=4),
                    Fu.FpMul(count=1, latency=4),
                    Fu.FpFma(count=1, latency=4),
                    Fu.FpDivSqrt(count=1, latency=15),
                    Fu.Branch(count=1, latency=1),
                    # One load AGU and one store AGU.
                    Fu.Mem(count=2, latency=1),
                ]
            ),
        ),
        branch_predictor=branch_predictor,  # type: ignore[arg-type]
        btb_size=32,  # 32-entry BTB, zero-bubble taken branches
        ras_size=16,  # 16-entry return stack
        cpu_clock_mhz=1400,  # EIC7700X
        misaligned_access_trap=True,  # no hardware misaligned access
        tlb_size=32,  # 32-entry fully associative L1 TLBs
        l2_tlb_size=512,
        ram_size=ram_size_bytes,
        # L1i: 32KB, 4-way, 64B lines (SiFive data sheet)
        l1i=Cache(
            size="32KB",
            line="64B",
            ways=4,
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=1),
        ),
        # L1d: 32KB, 4-way, 64B lines (SiFive data sheet); a hit is
        # one cycle of address generation plus `latency`.
        l1d=Cache(
            size="32KB",
            line="64B",
            ways=4,
            latency=2,  # 3-cycle load-to-use (Chips and Cheese)
            mshr_count=8,  # Non-blocking, modest MSHR count
        ),
        load_prefetcher=LoadPrefetcher.Stride(
            table_size=64,
            l1_lines=1,
            l2_lines=0,
            page_boundary=PageBoundary.Stop(),
        ),
        # Private L2 per core — size not publicly confirmed,
        # 256KB is consistent with area-optimized OoO cores
        # 256KB 8-way private L2 (data sheet); 13 cycles measured.
        l2=Cache(
            size="256KB",
            line="64B",
            ways=8,
            latency=12,
            mshr_count=16,
        ),
        # 4 MB L3 on the EIC7700X, ~38 cycles measured; single-core view.
        l3=Cache(
            size="4MB",
            line="64B",
            ways=16,
            latency=25,
            mshr_count=32,
        ),
        # LPDDR5-6400 on the HiFive Premier P550: tRCD, tRP and CL of about
        # 18 ns are 25 cycles each at 1.4 GHz; the rest of the measured
        # 194 ns (272 cycles) random-access latency is the SoC fabric and
        # controller, carried by the bus crossing each way.
        memory_controller=MemoryController.DRAM(
            t_cas=25,
            t_ras=25,
            t_pre=25,
        ),
        bus_latency=106,
    )


def rocket() -> Config:
    """Rocket as Chipyard 1.14.0's ``RocketConfig`` builds it: 1-wide
    in-order, 32 KiB L1s with a blocking L1D, 512 KiB L2.

    The structure comes from the RTL (rocket-chip 55bcad0 under Chipyard
    1.14.0); every latency the source does not state was measured on it
    with ``tools/diag/rtl_probe.py`` (see ``docs/rtl-rocket.md``):

    - ``WithNHugeCores`` (``rocket/Configs.scala``): ``MulDivParams(mulUnroll
      = 8, mulEarlyOut, divEarlyOut)``, ``FPUParams(minFLen = 16)`` with the
      default ``sfmaLatency = 3``, ``dfmaLatency = 4``; Zba, Zbb, Zbs.
      Measured: a dependent multiply delivers in 10 cycles, a 48-bit
      ``divu`` in about 53 (the divider skips leading zeros, so wider
      operands take up to 64), a dependent ``fadd.d`` in 5.
    - 32 KiB 8-way L1I (``ICacheParams(nSets = 64, nWays = 8)``, ``latency
      = 2``) and 32 KiB 8-way L1D (``DCacheParams(nSets = 64, nWays = 8,
      nMSHRs = 0)``, a blocking cache). Measured load-to-use: 2 cycles from
      the L1D, 18 from the L2, 24 to 29 from Chipyard's zero-latency memory
      model.
    - 28-entry BTB and 6-entry return stack (``BTBParams``); a 512-entry BHT
      of 1-bit counters with 8 bits of global history (``BHTParams``), which
      rvsim's ``GShare`` (4096 2-bit counters, 12 bits of history) stands in
      for. Measured: a mispredict costs 3 cycles.
    - 32-entry fully associative L1 TLBs and a 512-entry direct-mapped L2
      TLB (``nTLBWays``, ``nL2TLBEntries``); misaligned accesses trap.
    - 512 KiB 8-way inclusive L2 (``WithInclusiveCache`` defaults).
    - CSR accesses, exceptions, xRETs and the flush after a CSR write act in
      WB and redirect fetch the same cycle (``csr.io.rw`` and ``take_pc_wb``
      in ``rocket/RocketCore.scala``): no trap latency. Measured: the
      handler's first instruction retires 6 cycles after the instruction
      before a trapping ECALL, the instruction after an MRET 5 cycles after
      it, the one after a flushed CSR write 5, and the one after a FENCE.I
      21; a CSR read's dependent retires 3 cycles after it.
    """
    return Config(
        width=1,
        backend=Backend.InOrder(
            fu_config=Fu(
                [
                    Fu.IntAlu(count=1, latency=1),
                    Fu.IntMul(count=1, latency=10),
                    Fu.IntDiv(count=1, latency=52),
                    Fu.FpAdd(count=1, latency=6),
                    Fu.FpMul(count=1, latency=6),
                    Fu.FpFma(count=1, latency=6),
                    Fu.FpDivSqrt(count=1, latency=57),
                    Fu.Branch(count=1, latency=1),
                    Fu.Mem(count=1, latency=1),
                ]
            )
        ),
        branch_predictor=BranchPredictor.GShare(),
        btb_size=28,
        btb_ways=28,
        ras_size=6,
        redirect_latency=0,
        trap_latency=0,
        decode_rename_latency=0,
        rename_issue_latency=1,
        csr_squash="AffectingWrites",
        misaligned_access_trap=True,
        tlb_size=32,
        tlb_ways=0,
        l2_tlb_size=512,
        l2_tlb_ways=1,
        l1i=Cache(size="32KB", line="64B", ways=8, latency=1),
        l1d=Cache(size="32KB", line="64B", ways=8, latency=1, mshr_count=1),
        l2=Cache(size="512KB", line="64B", ways=8, latency=15),
        bus_latency=1,
        memory_controller=MemoryController.Simple(latency=1, bandwidth_gib_s=12.8),
    )


def boom() -> Config:
    """BOOM as Chipyard 1.14.0's ``MediumBoomV4Config`` builds it: 2-wide
    out-of-order, 64-entry ROB, 16 KiB L1s, 512 KiB L2.

    The structure comes from the RTL (riscv-boom 5223e44c under Chipyard
    1.14.0); every latency the source does not state was measured on it
    with ``tools/diag/rtl_probe.py`` (see ``docs/rtl-boom.md``):

    - ``WithNMediumBooms`` (``v4/common/config-mixins.scala``): ``fetchWidth
      = 4``, ``decodeWidth = 2``, ``numRobEntries = 64``, issue queues of 12
      (memory, 2-wide), 12 (unique), 20 (ALU, 2-wide) and 12 (FP) entries,
      ``numIntPhysRegisters = 80``, ``numFpPhysRegisters = 64``,
      ``numLdqEntries = 16``, ``numStqEntries = 16``, ``maxBrCount = 12``,
      ``FPUParams(sfmaLatency = 4, dfmaLatency = 4)``. Measured: a dependent
      multiply delivers in 7 cycles, a 48-bit ``divu`` in about 55, a
      dependent ``fadd.d`` in 5.
    - 16 KiB 4-way L1I (``ICacheParams(nSets = 64, nWays = 4)``) and 16 KiB
      4-way L1D (``DCacheParams(nSets = 64, nWays = 4, nMSHRs = 2, nTLBWays
      = 8)``). Measured load-to-use: 5 cycles from the L1D, 27 from the L2,
      33 to 46 from Chipyard's zero-latency memory model.
    - TAGE-L (``WithTAGELBPD``): six tagged tables of 128 or 256 sets by the
      4-wide fetch bank with histories 2, 4, 8, 16, 32 and 64 and tags of 7,
      7, 8, 8, 9 and 9 bits, useful bits reset every 2048 updates
      (``BoomTageParams``), over a 2048-set bimodal table
      (``BoomBIMParams``); a 128-set 2-way BTB (``BoomBTBParams``) and a
      32-entry return stack (``numRasEntries``). Measured: a mispredict
      costs 11 cycles.
    - 8-way L1 DTLB, 512-entry direct-mapped L2 TLB (``nL2TLBEntries``);
      misaligned accesses trap.
    - 512 KiB 8-way inclusive L2 (``WithInclusiveCache`` defaults).
    - Measured: the handler's first instruction retires 14 cycles after a
      trapping ECALL, the instruction after an MRET 13 cycles after it, the
      one after a CSR access 13 to 14, after a FENCE 13 and after a FENCE.I
      33.
    """
    return Config(
        width=2,
        fetch_width=4,
        backend=Backend.OutOfOrder(
            rob_size=64,
            issue_queue_size=56,
            load_queue_size=16,
            store_buffer_size=16,
            load_ports=1,
            store_ports=1,
            prf_gpr_size=80,
            prf_fpr_size=64,
            checkpoint_count=12,
            fu_config=Fu(
                [
                    Fu.IntAlu(count=2, latency=1),
                    Fu.IntMul(count=1, latency=7),
                    Fu.IntDiv(count=1, latency=55),
                    Fu.FpAdd(count=1, latency=5),
                    Fu.FpMul(count=1, latency=5),
                    Fu.FpFma(count=1, latency=5),
                    Fu.FpDivSqrt(count=1, latency=60),
                    Fu.Branch(count=2, latency=1),
                    Fu.Mem(count=1, latency=1),
                ]
            ),
        ),
        branch_predictor=BranchPredictor.TAGE(
            num_banks=6,
            table_size=1024,
            reset_interval=2048,
            history_lengths=[2, 4, 8, 16, 32, 64],
            tag_widths=[7, 7, 8, 8, 9, 9],
            bimodal_entries=8192,
        ),
        btb_size=256,
        btb_ways=2,
        ras_size=32,
        redirect_latency=2,
        trap_latency=4,
        fetch_decode_latency=2,
        csr_squash="EveryAccess",
        fence_squash=True,
        misaligned_access_trap=True,
        tlb_size=8,
        tlb_ways=0,
        l2_tlb_size=512,
        l2_tlb_ways=1,
        l1i=Cache(size="16KB", line="64B", ways=4, latency=2),
        l1d=Cache(size="16KB", line="64B", ways=4, latency=4, mshr_count=2),
        l2=Cache(size="512KB", line="64B", ways=8, latency=21, mshr_count=8),
        bus_latency=1,
        memory_controller=MemoryController.Simple(latency=1, bandwidth_gib_s=12.8),
    )


PRESETS = {
    "basic": basic,
    "fast": fast,
    "cortex_a72": cortex_a72,
    "m1": m1,
    "p550": p550,
    "rocket": rocket,
    "boom": boom,
}
