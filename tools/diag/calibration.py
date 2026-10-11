"""Configurations calibrated to measured hardware, for the hardware
comparisons.

Each is fitted to the published figures of one machine: its documented
structure sizes, the cache and memory latencies a pointer chase measures,
and, for the Cortex-A72, benchmark scores on a Raspberry Pi 4.
`latency_probe.py` and `linux_bench.py` run them against those figures,
and the validation page reports how close they come. They are not models
of the cores they are named after, whose RTL is not published, so they are
not offered as presets.
"""

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    LoadPrefetcher,
    MemoryController,
    PageBoundary,
    Prefetcher,
    StorePrefetcher,
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
        vlen=128,
        num_vec_lanes=None,
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
            prf_vpr_size=64,
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
        btb_ways=4,
        ras_size=31,
        cpu_clock_mhz=1500,  # Raspberry Pi 4
        clock_hz=1_000_000_000,
        ram_size="256MB",
        tlb_size=32,  # 32-entry L1 DTLB
        l2_tlb_size=1024,
        l2_tlb_ways=4,
        l2_tlb_latency=4,
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
        wcb_entries=0,
        # 1MB 16-way L2 on the BCM2711; 21 cycles measured on the A72.
        l2=Cache(
            size="1MB",
            line="64B",
            ways=16,
            latency=20,
            mshr_count=16,
        ),
        l3=None,
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
        vlen=128,
        num_vec_lanes=None,
        backend=Backend.OutOfOrder(
            rob_size=72,  # Modest, Core 2 / Goldmont Plus class
            issue_queue_size=32,  # "more scheduling capacity" than A75
            load_queue_size=24,  # "memory ordering queues can be a bit thin"
            store_buffer_size=16,  # Conservative estimate from "thin" description
            prf_gpr_size=128,  # "plenty of register file capacity compared to ROB"
            prf_fpr_size=96,  # Proportional to GPR, RV64GC needs FP regs
            prf_vpr_size=64,
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
        btb_ways=4,
        ras_size=16,  # 16-entry return stack
        cpu_clock_mhz=1400,  # EIC7700X
        clock_hz=1_000_000_000,
        misaligned_access_trap=True,  # no hardware misaligned access
        tlb_size=32,  # 32-entry fully associative L1 TLBs
        l2_tlb_size=512,
        l2_tlb_ways=4,
        l2_tlb_latency=4,
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
        store_prefetcher=None,
        wcb_entries=0,
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


#: The calibrated configurations by name, as the comparison tools take them.
CALIBRATED = {
    "cortex_a72": cortex_a72,
    "p550": p550,
}
