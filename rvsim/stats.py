"""
Simulation statistics container with pattern-based querying and comparison.

Provides ``Stats`` (dict subclass) with ``.query(pattern)`` for filtering,
``.compare(other)`` for two-way comparison, and ``.tabulate()`` for multi-run
tables.
"""

from __future__ import annotations

import math
import re
import sys
from collections.abc import Sequence
from enum import Enum
from typing import Any

__all__ = ["Stats", "Table"]


class Stats(dict):
    """
    Dict-like simulation statistics with querying and comparison.

    Keys are the simulator's stat paths (``core0.cache.l1d.misses``,
    ``core0.bp.committed.accuracy``, ``hart0.retired_insts``), plus the
    run-level ``cycles``, ``instructions_retired`` and ``ipc``.

    Example::

        result.stats["ipc"]
        result.stats["core0.pipeline.stalls.data"]
        result.stats.query("miss_rate")
    """

    def __init__(self, data: dict[str, Any]):
        super().__init__(data)

    @classmethod
    def from_core(cls, core: Any) -> Stats:
        """Flatten the native stats object into path-keyed entries.

        Every registered path (``core0.cache.l1d.hits``, ``system.retired_insts``)
        becomes a key, and the run-level ``cycles``, ``instructions_retired``
        and ``ipc`` are added under their short names.
        """
        data: dict[str, Any] = {
            path: _as_count_if_whole(path, core.get(path))
            for path in core.query("**").paths()
        }
        data["cycles"] = core.cycles
        data["instructions_retired"] = core.instructions_retired
        data["ipc"] = core.ipc
        return cls(data)

    def query(self, pattern: str) -> Stats:
        """Search for statistics matching *pattern* (case-insensitive regex or substring)."""
        matches = {}
        try:
            regex = re.compile(pattern, re.IGNORECASE)
        except re.error:
            regex = None

        for key, value in self.items():
            if regex:
                if regex.search(key):
                    matches[key] = value
            elif pattern.lower() in key.lower():
                matches[key] = value

        return Stats(matches)

    @staticmethod
    def tabulate(rows: dict[str, Stats], *, title: str = "") -> Table:
        """Build a comparison table from labeled `Stats` objects.

        Each *Stats* is typically a ``.query()`` result, so all share similar
        keys.  Columns are the sorted union of all keys across the provided
        Stats objects.

        Args:
            rows:  ``{label: Stats}`` — insertion order gives row order.
            title: Optional table title rendered above the header.

        Returns:
            `Table` with ``__repr__``/``__str__`` rendering.
        """
        if not rows:
            return Table([], [], [], title)

        labels = list(rows.keys())
        all_keys: set = set()
        for s in rows.values():
            all_keys.update(s.keys())
        metrics = sorted(all_keys)

        grid = []
        for label in labels:
            s = rows[label]
            grid.append([_fmt(s.get(m, "—")) for m in metrics])

        return Table(labels, metrics, grid, title)

    def compare(self, other: Stats) -> None:
        """Print a two-column comparison table (self vs other) to stdout."""
        all_keys = sorted(set(self) | set(other))
        if not all_keys:
            print("(no stats to compare)")
            return
        max_key = max(len(k) for k in all_keys)
        hdr = f"{'metric':<{max_key}}  {'self':>14}  {'other':>14}  {'diff':>14}"
        print(hdr)
        print("-" * len(hdr))
        for key in all_keys:
            v_self = self.get(key, "—")
            v_other = other.get(key, "—")
            diff = ""
            if isinstance(v_self, (int, float)) and isinstance(v_other, (int, float)):
                d = v_other - v_self
                if isinstance(d, float):
                    diff = f"{d:+.4f}"
                else:
                    diff = f"{d:+,}"
            print(
                f"{key:<{max_key}}  {_fmt(v_self):>14}  {_fmt(v_other):>14}  {diff:>14}"
            )

    def __repr__(self) -> str:
        if not self:
            return "Stats({})"
        items = sorted(self.items())
        key_w = max(len(k) for k in self.keys())
        val_w = max(len(_fmt(v)) for _, v in items)

        is_tty = hasattr(sys.stdout, "isatty") and sys.stdout.isatty()
        if not is_tty:
            lines = []
            for key, value in items:
                lines.append(f"{key:<{key_w}}  {_fmt(value):>{val_w}}")
            return "\n".join(lines)

        bold = "\033[1m"
        teal = "\033[36m"
        rst = "\033[0m"

        inner_w = key_w + 2 + val_w
        rule = f"{bold}{teal}{'─' * (inner_w + 4)}{rst}"

        parts = [rule]
        for key, value in items:
            parts.append(f"  {key:<{key_w}}  {_fmt(value):>{val_w}}")
        parts.append(rule)
        return "\n".join(parts)


# ── Formatting helpers ───────────────────────────────────────────────────────


def _as_count_if_whole(path: str, value: float) -> Any:
    """Counters come back from the core as floats; report them as ints."""
    if not _is_rate(path) and value.is_integer():
        return int(value)
    return value


def _fmt(v) -> str:
    if isinstance(v, float):
        return f"{v:.4f}"
    if isinstance(v, int):
        return f"{v:,}"
    return str(v)


def _weighted_harmonic_mean(values: Sequence[float], weights: Sequence[float]) -> float:
    """Weighted harmonic mean: sum(w) / sum(w/v). Skips zero values."""
    num = 0.0
    den = 0.0
    for v, w in zip(values, weights):
        if v > 0 and w > 0:
            num += w
            den += w / v
    if den == 0:
        return 0.0
    return num / den


def _geometric_mean(values: Sequence[float]) -> float:
    """Geometric mean via log. Skips non-positive values."""
    logs = [math.log(v) for v in values if v > 0]
    if not logs:
        return 0.0
    return math.exp(sum(logs) / len(logs))


_HEADLINE_METRICS = re.compile(
    r"^(cycles|instructions_retired|ipc"
    r"|core\d+\.bp\.committed\.accuracy"
    r"|core\d+\.cache\.(l1i|l1d|l2)\.miss_rate"
    r"|llc\.miss_rate)$"
)
"""Metrics a comparison shows when none are named."""

_RATE_LEAVES = frozenset({"ipc", "cpi", "accuracy", "miss_rate"})
_HIGHER_IS_BETTER_LEAVES = frozenset(
    {"ipc", "accuracy", "hits", "instructions_retired", "retired_insts"}
)
_LOWER_IS_BETTER_LEAVES = frozenset(
    {"cycles", "cpi", "miss_rate", "misses", "mispredicts"}
)
_LOWER_IS_BETTER_GROUPS = frozenset({"stalls", "flushes"})


class _Better(Enum):
    HIGHER = "higher"
    LOWER = "lower"


def _leaf(metric: str) -> str:
    return metric.rsplit(".", 1)[-1]


def _is_rate(metric: str) -> bool:
    return _leaf(metric) in _RATE_LEAVES


def _better(metric: str) -> _Better | None:
    """Which direction is an improvement for *metric*, if it has one."""
    leaf = _leaf(metric)
    if leaf in _HIGHER_IS_BETTER_LEAVES:
        return _Better.HIGHER
    if leaf in _LOWER_IS_BETTER_LEAVES:
        return _Better.LOWER
    if _LOWER_IS_BETTER_GROUPS & set(metric.split(".")[:-1]):
        return _Better.LOWER
    return None


def _sibling(metric: str, leaf: str) -> str:
    prefix = metric.rsplit(".", 1)[0]
    return f"{prefix}.{leaf}"


def _sum_of(stats_list: Sequence[Stats], key: str) -> float | None:
    values = [s.get(key) for s in stats_list]
    if any(not isinstance(v, (int, float)) for v in values):
        return None
    return float(sum(values))


def _aggregate_rate(metric: str, stats_list: Sequence[Stats]) -> float | None:
    """*metric* over several runs, as if they were one run.

    IPC and CPI are weighted by instructions retired; an accuracy or miss
    rate is recomputed from the hit and miss counters next to it.
    """
    leaf = _leaf(metric)
    if leaf == "accuracy":
        hits = _sum_of(stats_list, _sibling(metric, "hits"))
        misses = _sum_of(stats_list, _sibling(metric, "mispredicts"))
        return _ratio(hits, misses)
    if leaf == "miss_rate":
        misses = _sum_of(stats_list, _sibling(metric, "misses"))
        hits = _sum_of(stats_list, _sibling(metric, "hits"))
        return _ratio(misses, hits)

    values = [s.get(metric) for s in stats_list]
    weights = [s.get("instructions_retired", 0) for s in stats_list]
    if any(not isinstance(v, (int, float)) for v in values):
        return None
    if leaf == "ipc":
        return _weighted_harmonic_mean(values, weights)
    total_weight = sum(weights)
    if total_weight == 0:
        return None
    return sum(v * w for v, w in zip(values, weights)) / total_weight


def _ratio(numerator: float | None, other: float | None) -> float | None:
    """``numerator / (numerator + other)``, as the simulator derives it."""
    if numerator is None or other is None:
        return None
    total = numerator + other
    return numerator / total if total else 0.0


def _format_table(
    headers: list[str], rows: list[list[str]], align: list[str] | None = None
) -> str:
    """Render an ASCII table. align: list of '<' or '>' per column."""
    ncols = len(headers)
    if align is None:
        align = ["<"] + [">"] * (ncols - 1)
    widths = [len(h) for h in headers]
    for row in rows:
        for i, cell in enumerate(row):
            if i < ncols:
                widths[i] = max(widths[i], len(cell))
    parts = []
    hdr = "  ".join(f"{headers[i]:{align[i]}{widths[i]}}" for i in range(ncols))
    parts.append(hdr)
    parts.append("  ".join("-" * widths[i] for i in range(ncols)))
    for row in rows:
        line = "  ".join(
            f"{row[i]:{align[i]}{widths[i]}}" if i < len(row) else " " * widths[i]
            for i in range(ncols)
        )
        parts.append(line)
    return "\n".join(parts)


class Table:
    """Rendered comparison table.  Created by `tabulate`, displayed via
    ``print()`` or REPL auto-repr."""

    __slots__ = ("__col_header", "__grid", "__labels", "__metrics", "__title")

    def __dir__(self):
        return []

    def __init__(
        self,
        labels: list[str],
        metrics: list[str],
        grid: list[list[str]],
        title: str,
        col_header: str = "",
    ):
        self.__labels = labels
        self.__metrics = metrics
        self.__grid = grid
        self.__title = title
        self.__col_header = col_header

    def __repr__(self) -> str:
        return self.__render()

    def __str__(self) -> str:
        return self.__render()

    def __render(self) -> str:
        if not self.__labels:
            return "(empty table)"

        # Partition labels/rows into data rows and speedup rows.
        # Speedup rows are those that follow the sentinel row whose first cell
        # starts with "vs " (inserted by _compare_matrix).
        data_labels: list[str] = []
        data_grid: list[list[str]] = []
        speedup_labels: list[str] = []
        speedup_grid: list[list[str]] = []
        in_speedup = False
        for label, cells in zip(self.__labels, self.__grid):
            if label.startswith("baseline "):
                in_speedup = True
                speedup_labels.append(label)
                speedup_grid.append(cells)
            elif in_speedup:
                speedup_labels.append(label)
                speedup_grid.append(cells)
            else:
                data_labels.append(label)
                data_grid.append(cells)

        headers = [self.__col_header] + self.__metrics
        data_rows = [[label] + cells for label, cells in zip(data_labels, data_grid)]
        plain = _format_table(headers, data_rows)

        is_tty = hasattr(sys.stdout, "isatty") and sys.stdout.isatty()
        if not is_tty:
            if speedup_labels:
                spd_rows = [
                    [label] + cells
                    for label, cells in zip(speedup_labels, speedup_grid)
                ]
                plain += "\n" + _format_table(headers, spd_rows)
            return plain

        bold = "\033[1m"
        teal = "\033[36m"
        dim = "\033[2m"
        rst = "\033[0m"

        lines = plain.split("\n")
        width = max(len(line) for line in lines)
        rule = f"{bold}{teal}{'─' * (width + 4)}{rst}"

        parts = []
        if self.__title:
            # Prominent section header above the rule
            purple = "\033[35m"
            parts.append("")
            parts.append(f"  {bold}{purple}›  {self.__title}{rst}")
        parts.append(rule)
        parts.append(f"  {bold}{lines[0]}{rst}")
        parts.append(rule)
        for line in lines[2:]:
            parts.append(f"  {line}")
        if speedup_labels:
            dim_rule = f"{dim}{teal}{'─' * (width + 4)}{rst}"
            parts.append(dim_rule)
            spd_rows = [
                [label] + cells for label, cells in zip(speedup_labels, speedup_grid)
            ]
            spd_plain = _format_table(headers, spd_rows)
            for line in spd_plain.split("\n")[2:]:  # skip repeated header/rule
                parts.append(f"  {dim}{line}{rst}")
            parts.append(rule)
        else:
            parts.append(rule)

        return "\n".join(parts)


def _compare_flat(
    results: dict[str, Any],
    *,
    metrics: list[str] | None = None,
    baseline: str | None = None,
    col_header: str = "",
) -> None:
    """Compare single-binary, multiple-config results."""
    config_names = list(results.keys())
    if not config_names:
        print("(no results to compare)")
        return

    # Determine metrics to show
    all_stat_keys = set()
    for r in results.values():
        all_stat_keys.update(r.stats.keys())
    if metrics is not None:
        show_metrics = [m for m in metrics if m in all_stat_keys]
    else:
        show_metrics = sorted(k for k in all_stat_keys if _HEADLINE_METRICS.match(k))
        if not show_metrics:
            show_metrics = sorted(all_stat_keys)

    headers = [col_header or "metric"] + config_names
    rows: list[list[str]] = []
    for m in show_metrics:
        row = [m]
        for cfg_name in config_names:
            v = results[cfg_name].stats.get(m, "—")
            row.append(_fmt(v))
        rows.append(row)

    is_tty = hasattr(sys.stdout, "isatty") and sys.stdout.isatty()
    bold = "\033[1m"
    teal = "\033[36m"
    dim = "\033[2m"
    rst = "\033[0m"

    plain = _format_table(headers, rows)

    speedup_rows: list[list[str]] = []
    if baseline is not None and baseline in results:
        base_stats = results[baseline].stats
        for m in show_metrics:
            better = _better(m)
            if better is None:
                continue
            row = [m]
            bv = base_stats.get(m, 0)
            for cfg_name in config_names:
                v = results[cfg_name].stats.get(m, 0)
                if (
                    isinstance(bv, (int, float))
                    and isinstance(v, (int, float))
                    and bv != 0
                ):
                    ratio = bv / v if better is _Better.LOWER else v / bv
                    row.append(f"{ratio:.3f}x")
                else:
                    row.append("—")
            speedup_rows.append(row)

    if not is_tty:
        if speedup_rows and baseline is not None:
            plain += f"\n\nspeedup vs {baseline}\n"
            plain += _format_table(headers, speedup_rows)
        print(plain)
        return

    lines = plain.split("\n")
    width = max(len(line) for line in lines)
    rule = f"{bold}{teal}{'─' * (width + 4)}{rst}"

    parts = []
    parts.append(rule)
    parts.append(f"  {bold}{lines[0]}{rst}")
    parts.append(rule)
    for line in lines[2:]:
        parts.append(f"  {line}")
    parts.append(rule)

    if speedup_rows:
        dim_rule = f"{dim}{teal}{'─' * (width + 4)}{rst}"
        parts.append(dim_rule)
        spd_plain = _format_table(headers, speedup_rows)
        for line in spd_plain.split("\n")[2:]:
            parts.append(f"  {dim}{line}{rst}")
        parts.append(rule)
    else:
        parts.append(rule)

    print("\n".join(parts))


def _compare_matrix(
    results: dict[str, dict[str, Any]],
    *,
    metrics: list[str] | None = None,
    baseline: str | None = None,
    col_header: str = "",
) -> None:
    """Compare multi-binary x multi-config matrix."""
    binary_names = list(results.keys())
    config_names: list[str] = []
    for bdict in results.values():
        for k in bdict:
            if k not in config_names:
                config_names.append(k)

    if not config_names or not binary_names:
        print("(no results to compare)")
        return

    # Default to IPC + cycles if no metrics specified
    if metrics is None:
        metrics = ["ipc", "cycles"]

    for metric in metrics:
        labels: list[str] = []
        grid: list[list[str]] = []
        values_per_config: dict[str, list[float]] = {c: [] for c in config_names}

        for bname in binary_names:
            labels.append(bname)
            row: list[str] = []
            for cname in config_names:
                r = results[bname].get(cname)
                if r is None:
                    row.append("—")
                    continue
                v = r.stats.get(metric, "—")
                row.append(_fmt(v))
                if isinstance(v, (int, float)):
                    values_per_config[cname].append(float(v))
            grid.append(row)

        agg_cells: list[str] = []
        for cname in config_names:
            runs = [
                results[b][cname].stats for b in binary_names if cname in results[b]
            ]
            if not values_per_config[cname]:
                agg_cells.append("—")
            elif _is_rate(metric):
                aggregate = _aggregate_rate(metric, runs)
                agg_cells.append("—" if aggregate is None else f"{aggregate:.4f}")
            else:
                agg_cells.append(_fmt(int(sum(values_per_config[cname]))))
        labels.append("AGGREGATE")
        grid.append(agg_cells)

        better = _better(metric)
        show_speedup = (
            baseline is not None and baseline in config_names and better is not None
        )
        if show_speedup and baseline is not None:
            higher_is_better = better is _Better.HIGHER
            tag = "baseline " + baseline
            labels.append(tag)
            grid.append([""] * len(config_names))
            for bname in binary_names:
                r_base = results[bname].get(baseline)
                if r_base is None:
                    continue
                bv = r_base.stats.get(metric, 0)
                if not isinstance(bv, (int, float)) or bv == 0:
                    continue
                speedup_row: list[str] = []
                for cname in config_names:
                    r = results[bname].get(cname)
                    if r is None:
                        speedup_row.append("—")
                        continue
                    v = r.stats.get(metric, 0)
                    if isinstance(v, (int, float)) and v != 0:
                        ratio = (v / bv) if higher_is_better else (bv / v)
                        speedup_row.append(f"{ratio:.2f}x")
                    else:
                        speedup_row.append("—")
                labels.append(bname)
                grid.append(speedup_row)

            # Aggregate speedup: geomean of per-binary speedups
            agg_row: list[str] = []
            for cname in config_names:
                ratios: list[float] = []
                for bname in binary_names:
                    r_base = results[bname].get(baseline)
                    r = results[bname].get(cname)
                    if r_base is None or r is None:
                        continue
                    bv = r_base.stats.get(metric, 0)
                    v = r.stats.get(metric, 0)
                    if (
                        isinstance(bv, (int, float))
                        and isinstance(v, (int, float))
                        and bv != 0
                        and v != 0
                    ):
                        ratios.append((v / bv) if higher_is_better else (bv / v))
                if ratios:
                    agg_row.append(f"{_geometric_mean(ratios):.2f}x")
                else:
                    agg_row.append("—")
            labels.append("GEOMEAN")
            grid.append(agg_row)

        table = Table(labels, config_names, grid, metric, col_header=col_header)
        print(table)
