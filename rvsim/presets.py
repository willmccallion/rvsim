"""
Built-in configuration presets.

- ``basic`` — modest 4-wide OoO core, small caches, good for quick runs.
- ``fast``  — Apple M4 P-core class: 8-wide OoO at 4.4 GHz, 630-entry ROB,
  192KB L1I, 128KB L1D, 4MB L2, 36MB L3, the 64KB TAGE-SC-L with ITTAGE,
  4 unified FP/SIMD pipes, and DRAM controller.
- ``linux`` — a multi-core ``fast`` system with the memory map the
  bundled Linux image expects, kept coherent over an interconnect, with
  DDR5 memory.

Usage from the CLI::

    rvsim mandelbrot.elf --preset fast

Usage from Python::

    from rvsim import presets, Simulator
    cfg = presets.fast()
    Simulator(cfg, binary="mandelbrot.elf").run()
"""

from .config import Config
from .types import (
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

__all__ = ["basic", "fast", "linux", "PRESETS"]

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
) -> Config:
    """The ``fast`` core in a system that boots the bundled Linux image.

    ``harts`` harts boot through OpenSBI's HSM into an SMP kernel; with
    more than one, the private caches are kept coherent by a snoop-filter
    home agent over ``interconnect`` (``crossbar``, ``ring``, ``mesh``,
    ``torus`` or ``hypercube``). ``memory`` is ``ddr5`` (JEDEC
    command-level timing at ``speed_bin``, four channels) or ``dram`` (the
    ``fast`` preset's row-buffer model).

    With ``real_time`` the CLINT ticks at the device tree's 10 MHz
    timebase, so the guest's clock keeps time with the modelled one.
    Without it the CLINT ticks every cycle: guest time runs
    ``cpu_clock_mhz / 10`` times fast, which shortens a boot's sleeps and
    timeouts but floods measurements with timer interrupts.
    """
    base = fast()
    if interconnect not in INTERCONNECTS:
        raise ValueError(f"interconnect must be one of {sorted(INTERCONNECTS)}, got {interconnect!r}")
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
PRESETS = {
    "basic": basic,
    "fast": fast,
}
