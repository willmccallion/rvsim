#!/usr/bin/env python3
"""Probe the Linux boot hang: run for a small cycle budget, then dump state."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "setup"))

from rvsim import Simulator, presets

LIMIT = int(os.environ.get("PROBE_CYCLES", "10000000"))
PROGRESS = int(os.environ.get("PROBE_PROGRESS", "1000000"))

repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
image = os.path.join(repo, "software/linux/output/Image")
disk = os.path.join(repo, "software/linux/output/disk.img")

print(f"[probe] limit={LIMIT:,}  progress={PROGRESS:,}", flush=True)
sim = Simulator(presets.linux(), kernel=image, disk=disk)

exit_code = sim.run(limit=LIMIT, progress=PROGRESS, stats_sections=None)

print()
print(f"[probe] exit_code={exit_code}")
print(f"[probe] pc=0x{sim.pc:x}")
print(f"[probe] privilege={sim.privilege}")
if hasattr(sim, "stats"):
    s = dict(sim.stats)
    keys = sorted(k for k in s if "instructions" in k.lower() or "cycle" in k.lower() or "stall" in k.lower())
    for k in keys[:25]:
        print(f"  {k} = {s[k]}")
