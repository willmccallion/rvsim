"""
Run gem5 on a single binary. Invoked by run_gem5.py as a subprocess.

Usage:
    gem5.opt scripts/comparison/gem5_single.py <binary.elf> <m5out_dir>
"""

import sys
from pathlib import Path

from gem5.components.boards.simple_board import SimpleBoard
from gem5.components.cachehierarchies.classic.private_l1_private_l2_cache_hierarchy import (
    PrivateL1PrivateL2CacheHierarchy,
)
from gem5.components.memory.single_channel import SingleChannelDDR3_1600
from gem5.components.processors.base_cpu_core import BaseCPUCore
from gem5.components.processors.base_cpu_processor import BaseCPUProcessor
from gem5.isas import ISA
from gem5.resources.resource import BinaryResource
from gem5.simulate.simulator import Simulator
from gem5.utils.requires import requires
from m5.objects import (
    FUPool, FP_ALU, FP_MultDiv, IntALU, IntMultDiv, ReadPort,
    RiscvO3CPU, TournamentBP, WritePort,
)

requires(isa_required=ISA.RISCV)

binary = Path(sys.argv[1])
m5out = sys.argv[2]

# Redirect gem5 output directory
import m5
m5.options.outdir = m5out


class P550Core(BaseCPUCore):
    def __init__(self, core_id: int):
        requires(isa_required=ISA.RISCV)
        fu_pool = FUPool(FUList=[
            IntALU(count=3),
            IntMultDiv(count=1),
            FP_ALU(count=2),
            FP_MultDiv(count=2),
            ReadPort(count=1),
            WritePort(count=1),
        ])
        cpu = RiscvO3CPU(
            fuPool=fu_pool,
            cpu_id=core_id,
            branchPred=TournamentBP(),
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


class P550Processor(BaseCPUProcessor):
    def __init__(self):
        super().__init__(cores=[P550Core(core_id=0)])


processor = P550Processor()
cache_hierarchy = PrivateL1PrivateL2CacheHierarchy(
    l1d_size="32KiB",
    l1i_size="32KiB",
    l2_size="256KiB",
)
memory = SingleChannelDDR3_1600(size="256MiB")
board = SimpleBoard(
    clk_freq="1.4GHz",
    processor=processor,
    memory=memory,
    cache_hierarchy=cache_hierarchy,
)
board.set_se_binary_workload(BinaryResource(local_path=str(binary)))

sim = Simulator(board=board, full_system=False)
sim.run()
