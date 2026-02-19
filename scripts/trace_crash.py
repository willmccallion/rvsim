"""Trace the last instructions before qsort crashes."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from rvsim import Config, Disassemble

binary = sys.argv[1] if len(sys.argv) > 1 else "software/bin/programs/qsort.bin"

# Enable trace to see pipeline state
config = Config(trace=True)
config_dict = config.to_dict()

sys_obj = System(config)
with open(binary, "rb") as f:
    sys_obj.load_binary(f.read(), 0x80000000)

cpu = Cpu(sys_obj, config_dict)

# Run until crash — trace goes to stderr
exit_code = cpu.run()
print(f"Exit code: {exit_code}", file=sys.stdout)

trace = cpu.get_pc_trace()
print(f"\nLast 10 retired instructions:", file=sys.stdout)
for pc, raw in trace[-10:]:
    asm = Disassemble().inst(raw)
    print(f"  0x{pc:08x}  {asm}", file=sys.stdout)

from rvsim import reg_name

print(f"\nRegisters:", file=sys.stdout)
for i in range(32):
    val = cpu.read_register(i)
    if val != 0:
        print(f"  x{i:<2} ({reg_name(i):>4}) = 0x{val:016x}", file=sys.stdout)
