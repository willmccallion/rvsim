"""Live TUI dashboard for --watch mode."""

from __future__ import annotations

import os
import sys
import tempfile
import time
from typing import Optional

from rich.console import Console
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text


# Cycles simulated per chunk between renders. Larger = faster sim, less responsive UI.
_CHUNK = 200_000
_BAR = 8


def _bar(ratio: float) -> Text:
    filled = max(0, min(_BAR, round(ratio * _BAR)))
    s = "█" * filled + "░" * (_BAR - filled)
    color = "green" if ratio >= 0.95 else "yellow" if ratio >= 0.75 else "red"
    return Text(s, style=color)


def _fmt(n: int) -> str:
    if n >= 1_000_000_000:
        return f"{n / 1_000_000_000:.2f}G"
    if n >= 1_000_000:
        return f"{n / 1_000_000:.2f}M"
    if n >= 1_000:
        return f"{n / 1_000:.1f}K"
    return str(n)


def _section(title: str, rows: list[tuple]) -> Panel:
    t = Table(show_header=False, box=None, padding=(0, 1), expand=True)
    t.add_column(style="dim")
    t.add_column(justify="right")
    t.add_column()
    for row in rows:
        t.add_row(*row)
    return Panel(t, title=f"[bold]{title}[/]", border_style="cyan")


def _build(stats: dict, wall: float, binary: str, done: bool) -> Table:
    s = stats

    cycles = s.get("cycles", 0)
    ipc = s.get("ipc", 0.0)
    retired = s.get("instructions_retired", 0)
    branch_acc = s.get("branch_accuracy_pct", 0.0)
    branch_n = s.get("branch_predictions", 0)
    branch_mis = s.get("branch_mispredictions", 0)
    stall_mem = s.get("stalls_mem", 0)
    stall_ctrl = s.get("stalls_control", 0)
    stall_data = s.get("stalls_data", 0)

    def hit_rate(hits, misses):
        t = hits + misses
        return hits / t if t else 1.0

    l1i_r = hit_rate(s.get("icache_hits", 0), s.get("icache_misses", 0))
    l1d_r = hit_rate(s.get("dcache_hits", 0), s.get("dcache_misses", 0))
    l2_r = hit_rate(s.get("l2_hits", 0), s.get("l2_misses", 0))

    def sr(n):
        return n / cycles if cycles else 0.0

    core = _section(
        "core",
        [
            ("cycles", _fmt(cycles), ""),
            ("retired", _fmt(retired), ""),
            ("IPC", f"{ipc:.3f}", _bar(min(ipc / 4.0, 1.0))),
        ],
    )
    branch = _section(
        "branch",
        [
            ("accuracy", f"{branch_acc:.2f}%", _bar(branch_acc / 100)),
            ("lookups", _fmt(branch_n + branch_mis), ""),
            ("mispredicts", _fmt(branch_mis), ""),
        ],
    )
    cache = _section(
        "cache",
        [
            ("L1i", f"{l1i_r * 100:.2f}%", _bar(l1i_r)),
            ("L1d", f"{l1d_r * 100:.2f}%", _bar(l1d_r)),
            ("L2", f"{l2_r * 100:.2f}%", _bar(l2_r)),
        ],
    )
    stalls = _section(
        "stalls",
        [
            ("memory", f"{sr(stall_mem) * 100:.1f}%", _bar(sr(stall_mem))),
            ("control", f"{sr(stall_ctrl) * 100:.1f}%", _bar(sr(stall_ctrl))),
            ("data", f"{sr(stall_data) * 100:.1f}%", _bar(sr(stall_data))),
        ],
    )

    # Lay the four panels out as columns in a grid table
    grid = Table.grid(expand=True)
    grid.add_column(ratio=1)
    grid.add_column(ratio=1)
    grid.add_column(ratio=1)
    grid.add_column(ratio=1)
    grid.add_row(core, branch, cache, stalls)

    status = "[bold green]done[/]" if done else "[bold yellow]running[/]"
    return Panel(
        grid,
        title=f"[bold cyan]{binary}[/]  {status}  [dim]{wall:.1f}s wall[/]",
        border_style="bright_black",
    )


def run_watch(
    cpu, limit: Optional[int], binary: str, print_stats: bool = False
) -> Optional[int]:
    """Run *cpu* with a live-updating dashboard. Returns exit code."""
    console = Console()
    start = time.monotonic()
    cycles_run = 0
    exit_code = None

    # Capture all program output (UART writes to fd 2 via Rust eprint!) into a
    # temp file so it doesn't interleave with the TUI. We redirect at the OS
    # file-descriptor level so the Rust side is also captured.
    stderr_fd = sys.stderr.fileno()
    saved_stderr_fd = os.dup(stderr_fd)
    tmp = tempfile.TemporaryFile()
    os.dup2(tmp.fileno(), stderr_fd)

    live = Live(console=console, refresh_per_second=4, screen=False)
    live.start()
    try:
        while True:
            chunk = _CHUNK
            if limit is not None:
                remaining = limit - cycles_run
                if remaining <= 0:
                    break
                chunk = min(chunk, remaining)

            code = cpu.run(limit=chunk, stats_sections=None)
            cycles_run += chunk

            if code is not None:
                exit_code = code

            wall = time.monotonic() - start
            live.update(_build(dict(cpu.stats), wall, binary, exit_code is not None))

            if exit_code is not None:
                # Render the final "done" frame explicitly, then stop before
                # the Live context can emit a second render on __exit__.
                live.refresh()
                break
    finally:
        live.stop()
        # Restore stderr fd before printing captured output.
        os.dup2(saved_stderr_fd, stderr_fd)
        os.close(saved_stderr_fd)

    # Print any program output that was buffered during the run.
    tmp.seek(0)
    program_output = tmp.read()
    tmp.close()
    if program_output:
        sys.stdout.buffer.write(program_output)
        sys.stdout.buffer.flush()

    if print_stats and exit_code is not None:
        cpu.run(limit=0, stats_sections=[])

    return exit_code
