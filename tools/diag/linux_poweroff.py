#!/usr/bin/env python3
"""Boot Linux to the login prompt, log in, power off, and check it went cleanly.

Boots the system ``make run-linux`` boots (``presets.linux`` with eight harts,
a mesh and DDR5 by default), answers the login prompt with ``root``, runs
``poweroff`` at the shell, and exits 0 once the kernel powers the machine
down. A kernel panic, an oops, a BUG, a run that ends any other way, or no
power-down within ``--timeout`` seconds (or ``--limit`` simulated cycles)
exits 1. The console goes to stdout.

The timeout is in wall-clock time because simulated time is no measure of
a hang: idle cycles are skipped, so a boot spends billions of them waiting.

Usage:
    python tools/diag/linux_poweroff.py
    python tools/diag/linux_poweroff.py --harts 1 --timeout 3600
"""

import argparse
import os
import re
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, ROOT)

from rvsim import Simulator, presets

CHUNK_CYCLES = 5_000_000
LOGIN = re.compile(r"login: $")
SHELL = re.compile(r"# $")
POWERED_DOWN = "reboot: Power down"
FAILURES = re.compile(r"Kernel panic|Oops|BUG:|Unable to handle kernel")


def boot_to_poweroff(sim: Simulator, deadline: float, limit: int | None) -> str | None:
    """Drives `sim` from reset to power-down before `deadline` (a
    `time.monotonic` value) and, if given, cycle `limit`; returns why it
    failed, or None."""
    console = ""
    logged_in = powering_off = False
    while time.monotonic() < deadline and (limit is None or sim.cycle < limit):
        chunk = CHUNK_CYCLES if limit is None else min(CHUNK_CYCLES, limit - sim.cycle)
        reason, _ = sim.run_to(cycles=chunk, console_output=True)
        text = sim.read_console()
        console += text
        sys.stdout.write(text)
        sys.stdout.flush()
        failure = FAILURES.search(console)
        if failure:
            return f"the kernel reported {failure.group(0)!r}"
        if reason == "exit":
            if POWERED_DOWN in console:
                return None
            return "the run ended without powering down"
        if not logged_in and LOGIN.search(console):
            sim.write_console("root\n")
            logged_in = True
        elif logged_in and not powering_off and SHELL.search(console):
            sim.write_console("poweroff\n")
            powering_off = True
    stage = "power-down" if powering_off else "shell" if logged_in else "login prompt"
    return f"no {stage} in time"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--harts", type=int, default=8)
    ap.add_argument("--timeout", type=float, default=3 * 60 * 60, help="seconds")
    ap.add_argument("--limit", type=int, default=None, help="simulated cycles")
    args = ap.parse_args()

    out_dir = os.path.join(ROOT, "software", "linux", "output")
    kernel = os.path.join(out_dir, "Image")
    disk = os.path.join(out_dir, "disk.img")
    if not (os.path.exists(kernel) and os.path.exists(disk)):
        print(f"no Linux image in {out_dir}; run `make linux`", file=sys.stderr)
        return 1

    os.chdir(ROOT)
    config = presets.linux(args.harts, real_time=False).replace(console="captured")
    sim = Simulator(config, kernel=kernel, disk=disk)
    started = time.monotonic()
    failure = boot_to_poweroff(sim, started + args.timeout, args.limit)
    elapsed = time.monotonic() - started
    summary = f"{args.harts} hart(s), {sim.cycle:,} cycles, {elapsed:,.0f} s"
    if failure is not None:
        print(f"\nlinux_poweroff: FAILED after {summary}: {failure}", file=sys.stderr)
        return 1
    print(f"\nlinux_poweroff: booted, logged in and powered down ({summary})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
