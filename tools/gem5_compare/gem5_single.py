"""
Run gem5 on a single binary. Invoked by run_gem5.py as a subprocess.

Usage:
    gem5.opt tools/gem5_compare/gem5_single.py <binary.elf> <m5out_dir> <variant>

<variant> names an entry of variants.VARIANTS; the machine is built from it
exactly as run_rvsim.py builds rvsim's.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from gem5.components.boards.simple_board import SimpleBoard
from gem5.components.cachehierarchies.classic.caches.l1dcache import L1DCache
from gem5.components.cachehierarchies.classic.caches.l1icache import L1ICache
from gem5.components.cachehierarchies.classic.caches.l2cache import L2Cache
from gem5.components.cachehierarchies.classic.private_l1_private_l2_cache_hierarchy import (
    PrivateL1PrivateL2CacheHierarchy,
)
from gem5.components.memory.simple import SingleChannelSimpleMemory
from gem5.components.processors.base_cpu_core import BaseCPUCore
from gem5.components.processors.base_cpu_processor import BaseCPUProcessor
from gem5.isas import ISA
from gem5.resources.resource import BinaryResource
from gem5.simulate.simulator import Simulator
from gem5.utils.requires import requires
from m5.objects import (
    BadAddr, FUPool, FP_ALU, FP_MultDiv, IntALU, IntMultDiv, L2XBar, ReadPort,
    RiscvO3CPU, SIMD_Unit, SystemXBar, TAGE, TAGE_SC_L_64KB, TAGEBase, TaggedPrefetcher,
    TournamentBP, WritePort,
)
from m5.params import NULL

from variants import CLOCK_MHZ, VARIANTS, VECTOR_UNITS, VLEN

# One core clock in ticks (picoseconds), as gem5 rounds the clock period.
CYCLE_PS = int(1e6 / CLOCK_MHZ)

requires(isa_required=ISA.RISCV)

binary = Path(sys.argv[1])
m5out = sys.argv[2]
variant = VARIANTS[sys.argv[3]]

import m5
m5.options.outdir = m5out


def branch_predictor(bp: dict):
    """The variant's predictor. Indirect targets come from the BTB and a
    taken prediction needs a BTB hit, as in rvsim's front end, where fetch
    knows a branch only through the BTB and decode redirects for the rest."""
    common = dict(indirectBranchPred=NULL, requiresBTBHit=True)
    if bp["kind"] == "tournament":
        return TournamentBP(
            localPredictorSize=2 ** bp["local_pred_bits"],
            localHistoryTableSize=2 ** bp["local_hist_bits"],
            globalPredictorSize=2 ** bp["global_bits"],
            choicePredictorSize=2 ** bp["global_bits"],
            **common,
        )
    if bp["kind"] == "tage":
        return TAGE(
            tage=TAGEBase(
                nHistoryTables=bp["tables"],
                minHist=bp["min_hist"],
                maxHist=bp["max_hist"],
                tagTableTagWidths=[0] + bp["tag_widths"],
                logTagTableSizes=[bp["log_table_size"]] * (bp["tables"] + 1),
                logUResetPeriod=bp["log_u_reset"],
            ),
            **common,
        )
    if bp["kind"] == "tage_sc_l":
        return TAGE_SC_L_64KB(**common)
    raise ValueError(f"unknown predictor {bp['kind']}")


class P550Core(BaseCPUCore):
    def __init__(self, core_id: int):
        requires(isa_required=ISA.RISCV)
        fu_pool = FUPool(FUList=[
            IntALU(count=3),
            IntMultDiv(count=1),
            FP_ALU(count=2),
            FP_MultDiv(count=2),
            SIMD_Unit(count=VECTOR_UNITS),
            ReadPort(count=1),
            WritePort(count=1),
        ])
        cpu = RiscvO3CPU(
            fuPool=fu_pool,
            cpu_id=core_id,
            branchPred=branch_predictor(variant["bp"]),
            numROBEntries=72,
            numIQEntries=32,
            numPhysIntRegs=128,
            numPhysFloatRegs=96,
            LQEntries=24,
            SQEntries=16,
            fetchWidth=3,
            decodeWidth=3,
            renameWidth=3,
            dispatchWidth=3,
            issueWidth=3,
            wbWidth=3,
            commitWidth=3,
        )
        super().__init__(core=cpu, isa=ISA.RISCV)
        self.core.isa[0].vlen = VLEN


class P550Processor(BaseCPUProcessor):
    def __init__(self):
        super().__init__(cores=[P550Core(core_id=0)])


def no_prefetcher():
    return NULL


class VariantCacheHierarchy(PrivateL1PrivateL2CacheHierarchy):
    """The stdlib private L1/L2 hierarchy with every cache built from the
    variant: its size, associativity, MSHRs and targets, and no prefetcher
    unless the variant asks for the L1D's."""

    def incorporate_cache(self, board) -> None:
        board.connect_system_port(self.membus.cpu_side_ports)
        for _, port in board.get_memory().get_mem_ports():
            self.membus.mem_side_ports = port
        # rvsim's L1s talk to their L2 directly: no crossbar delay.
        self.l2buses = [
            L2XBar(width=64, frontend_latency=0, forward_latency=0, response_latency=0)
            for _ in range(board.get_processor().get_num_cores())
        ]
        degree = variant["l1d_prefetch_degree"]
        l1d_prefetcher = (lambda: TaggedPrefetcher(degree=degree)) if degree else no_prefetcher
        for i, cpu in enumerate(board.get_processor().get_cores()):
            l2 = variant["l2"]
            l2_node = self.add_root_child(f"l2-cache-{i}", L2Cache(
                size=f"{l2['size_kb']}KiB", assoc=l2["assoc"], mshrs=l2["mshrs"],
                tag_latency=l2["latency"], data_latency=l2["latency"],
                response_latency=l2["response"],
                tgts_per_mshr=l2["tgts"], PrefetcherCls=no_prefetcher,
            ))
            l1i = variant["l1i"]
            l1i_node = l2_node.add_child(f"l1i-cache-{i}", L1ICache(
                size=f"{l1i['size_kb']}KiB", assoc=l1i["assoc"], mshrs=l1i["mshrs"],
                tag_latency=l1i["latency"], data_latency=l1i["latency"],
                response_latency=l1i["response"],
                tgts_per_mshr=l1i["tgts"], PrefetcherCls=no_prefetcher,
            ))
            l1d = variant["l1d"]
            l1d_node = l2_node.add_child(f"l1d-cache-{i}", L1DCache(
                size=f"{l1d['size_kb']}KiB", assoc=l1d["assoc"], mshrs=l1d["mshrs"],
                tag_latency=l1d["latency"], data_latency=l1d["latency"],
                response_latency=l1d["response"],
                tgts_per_mshr=l1d["tgts"], PrefetcherCls=l1d_prefetcher,
            ))
            self.l2buses[i].mem_side_ports = l2_node.cache.cpu_side
            self.membus.cpu_side_ports = l2_node.cache.mem_side
            l1i_node.cache.mem_side = self.l2buses[i].cpu_side_ports
            l1d_node.cache.mem_side = self.l2buses[i].cpu_side_ports
            cpu.connect_icache(l1i_node.cache.cpu_side)
            cpu.connect_dcache(l1d_node.cache.cpu_side)
            self._connect_table_walker(i, cpu)
            cpu.connect_interrupt()


processor = P550Processor()
bus = variant["bus"]
membus = SystemXBar(
    width=bus["width_bytes"],
    frontend_latency=bus["latency"],
    forward_latency=0,
    response_latency=bus["latency"],
)
membus.badaddr_responder = BadAddr()
membus.default = membus.badaddr_responder.pio
cache_hierarchy = VariantCacheHierarchy(
    l1d_size=f"{variant['l1d']['size_kb']}KiB",
    l1i_size=f"{variant['l1i']['size_kb']}KiB",
    l2_size=f"{variant['l2']['size_kb']}KiB",
    membus=membus,
)
memory = SingleChannelSimpleMemory(
    latency=f"{variant['memory']['latency'] * CYCLE_PS}ps",
    latency_var="0ns",
    bandwidth=f"{variant['memory']['bandwidth_gib_s']}GiB/s",
    size="256MiB",
)
board = SimpleBoard(
    clk_freq=f"{CLOCK_MHZ}MHz",
    processor=processor,
    memory=memory,
    cache_hierarchy=cache_hierarchy,
)
board.set_se_binary_workload(BinaryResource(local_path=str(binary)))

sim = Simulator(board=board, full_system=False)
sim.run()
