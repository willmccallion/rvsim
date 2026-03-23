#!/usr/bin/env python3
"""Debug vector instruction execution with tracing.

Usage:
    rvsim scripts/debug_vec.py
"""

import sys
from rvsim import Backend, Config, Simulator

cfg = Config(width=4, backend=Backend.OutOfOrder())
cpu = Simulator().config(cfg).binary("software/bin/programs/maze.elf").build()
cpu.trace = True
rc = cpu.run(limit=1200)
stats = cpu.stats
print(f"\nexit code: {rc}", file=sys.stderr)
print(f"cycles: {stats['cycles']}", file=sys.stderr)
print(f"retired: {stats['instructions_retired']}", file=sys.stderr)
print(f"pc: {cpu.pc:#x}", file=sys.stderr)

# Show last committed instructions around where it stalls
trace = cpu.pc_trace
print(f"\nlast {min(30, len(trace))} committed instructions:", file=sys.stderr)
for pc, inst in trace[-30:]:
    print(f"  {pc:#010x}: {inst:#010x}", file=sys.stderr)
