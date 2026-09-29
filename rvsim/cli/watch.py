"""Live TUI dashboard for --watch mode."""

from __future__ import annotations

import os
import sys
import tempfile
import time
from typing import IO

from rich.console import Console
from rich.live import Live
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from rvsim.stats import Stats

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
    branch_acc = s.get("core0.bp.committed.accuracy", 0.0)
    branch_hits = s.get("core0.bp.committed.hits", 0)
    branch_mis = s.get("core0.bp.committed.mispredicts", 0)
    stall_fu = s.get("core0.pipeline.stalls.fu_structural", 0)
    stall_ctrl = s.get("core0.pipeline.stalls.control", 0)
    stall_data = s.get("core0.pipeline.stalls.data", 0)

    def hit_rate(cache: str) -> float:
        return 1.0 - s.get(f"core0.cache.{cache}.miss_rate", 0.0)

    l1i_r = hit_rate("l1i")
    l1d_r = hit_rate("l1d")
    l2_r = hit_rate("l2")

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
            ("accuracy", f"{branch_acc * 100:.2f}%", _bar(branch_acc)),
            ("lookups", _fmt(branch_hits + branch_mis), ""),
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
            ("FUs", f"{sr(stall_fu) * 100:.1f}%", _bar(sr(stall_fu))),
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
    cpu, limit: int | None, binary: str, print_stats: bool = False
) -> int | None:
    """Run *cpu* with a live-updating dashboard. Returns exit code."""
    with tempfile.TemporaryFile() as captured:
        exit_code = _run_live(cpu, limit, binary, captured)
        captured.seek(0)
        program_output = captured.read()
    if program_output:
        sys.stdout.buffer.write(program_output)
        sys.stdout.buffer.flush()

    if print_stats and exit_code is not None:
        cpu.run(limit=0, stats_sections=[])

    return exit_code


def _run_live(cpu, limit: int | None, binary: str, captured: IO[bytes]) -> int | None:
    """Runs *cpu* under the dashboard with fd 2 redirected into *captured*.

    UART output is written to fd 2 from Rust, so the redirect is at the file
    descriptor level; it keeps program output from interleaving with the TUI.
    """
    console = Console()
    start = time.monotonic()
    cycles_run = 0
    exit_code = None

    stderr_fd = sys.stderr.fileno()
    saved_stderr_fd = os.dup(stderr_fd)
    os.dup2(captured.fileno(), stderr_fd)

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
            stats = Stats.from_core(cpu.stats)
            live.update(_build(stats, wall, binary, exit_code is not None))

            if exit_code is not None:
                # Render the final "done" frame explicitly, then stop before
                # the Live context can emit a second render on __exit__.
                live.refresh()
                break
    finally:
        live.stop()
        os.dup2(saved_stderr_fd, stderr_fd)
        os.close(saved_stderr_fd)
    return exit_code
