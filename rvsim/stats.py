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
from typing import Any, Dict, List, Optional, Sequence, Union

__all__ = ["Stats", "Table"]


class Stats(dict):
    """
    Dict-like simulation statistics with querying and comparison.

    All stats from the backend are accessible as keys. Typical keys include:
    cycles, instructions_retired, ipc, icache_hits, icache_misses, dcache_hits,
    dcache_misses, l2_hits, l2_misses, l3_hits, l3_misses, stalls_mem, stalls_control,
    stalls_data, branch_predictions, branch_mispredictions, branch_accuracy_pct, etc.

    Example::

        result.stats["ipc"]
        result.stats.query("miss")
        result.stats.query("branch")
    """

    def __init__(self, data: Dict[str, Any]):
        super().__init__(data)

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
    def tabulate(rows: Dict[str, Stats], *, title: str = "") -> Table:
        """Build a comparison table from labeled :class:`Stats` objects.

        Each *Stats* is typically a ``.query()`` result, so all share similar
        keys.  Columns are the sorted union of all keys across the provided
        Stats objects.

        Args:
            rows:  ``{label: Stats}`` — insertion order gives row order.
            title: Optional table title rendered above the header.

        Returns:
            :class:`Table` with ``__repr__``/``__str__`` rendering.
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


_RATE_METRICS = {"ipc", "branch_accuracy_pct", "speculative_branch_accuracy_pct"}
_COUNT_METRICS = {
    "cycles",
    "instructions_retired",
    "stalls_mem",
    "stalls_control",
    "stalls_data",
    "stalls_fu_structural",
    "stalls_backpressure",
    "misprediction_penalty",
    "pipeline_flushes",
    "mem_ordering_violations",
    "icache_hits",
    "icache_misses",
    "dcache_hits",
    "dcache_misses",
    "l2_hits",
    "l2_misses",
    "l3_hits",
    "l3_misses",
    "branch_predictions",
    "branch_mispredictions",
    "committed_branch_predictions",
    "committed_branch_mispredictions",
    "speculative_branch_predictions",
    "speculative_branch_mispredictions",
    "traps_taken",
    "inst_load",
    "inst_store",
    "inst_branch",
    "inst_alu",
    "inst_system",
    "inst_fp_load",
    "inst_fp_store",
    "inst_fp_arith",
    "inst_fp_fma",
    "inst_fp_div_sqrt",
}


def _format_table(
    headers: List[str], rows: List[List[str]], align: Optional[List[str]] = None
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
    """Rendered comparison table.  Created by :func:`tabulate`, displayed via
    ``print()`` or REPL auto-repr."""

    __slots__ = ("__labels", "__metrics", "__grid", "__title", "__col_header")

    def __dir__(self):
        return []

    def __init__(
        self,
        labels: List[str],
        metrics: List[str],
        grid: List[List[str]],
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
        data_labels: List[str] = []
        data_grid: List[List[str]] = []
        speedup_labels: List[str] = []
        speedup_grid: List[List[str]] = []
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
    results: Dict[str, Any],
    *,
    metrics: Optional[List[str]] = None,
    baseline: Optional[str] = None,
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
        show_metrics = sorted(all_stat_keys & (_RATE_METRICS | _COUNT_METRICS))
        if not show_metrics:
            show_metrics = sorted(all_stat_keys)

    headers = [col_header or "metric"] + config_names
    rows: List[List[str]] = []
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

    speedup_rows: List[List[str]] = []
    if baseline is not None and baseline in results:
        base_stats = results[baseline].stats
        for m in show_metrics:
            if m not in _RATE_METRICS and m != "cycles":
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
                    if m == "cycles":
                        ratio = bv / v  # lower is better
                    else:
                        ratio = v / bv  # higher is better
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
    results: Dict[str, Dict[str, Any]],
    *,
    metrics: Optional[List[str]] = None,
    baseline: Optional[str] = None,
    col_header: str = "",
) -> None:
    """Compare multi-binary x multi-config matrix."""
    binary_names = list(results.keys())
    config_names: List[str] = []
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
        labels: List[str] = []
        grid: List[List[str]] = []
        values_per_config: Dict[str, List[float]] = {c: [] for c in config_names}
        weights_per_config: Dict[str, List[float]] = {c: [] for c in config_names}

        for bname in binary_names:
            labels.append(bname)
            row: List[str] = []
            for cname in config_names:
                r = results[bname].get(cname)
                if r is None:
                    row.append("—")
                    continue
                v = r.stats.get(metric, "—")
                row.append(_fmt(v))
                if isinstance(v, (int, float)):
                    values_per_config[cname].append(float(v))
                    inst = r.stats.get("instructions_retired", 1)
                    weights_per_config[cname].append(float(inst))
            grid.append(row)

        # Aggregate row
        is_rate = metric in _RATE_METRICS
        agg_cells: List[str] = []
        for cname in config_names:
            vals = values_per_config[cname]
            wgts = weights_per_config[cname]
            if not vals:
                agg_cells.append("—")
            elif is_rate:
                agg_cells.append(f"{_weighted_harmonic_mean(vals, wgts):.4f}")
            else:
                agg_cells.append(_fmt(int(sum(vals))))
        labels.append("AGGREGATE")
        grid.append(agg_cells)

        # Baseline speedup rows — only for metrics with clear directionality
        _LOWER_IS_BETTER = {
            "cycles",
            "stalls_mem",
            "stalls_control",
            "stalls_data",
            "icache_misses",
            "dcache_misses",
            "l2_misses",
            "l3_misses",
            "branch_mispredictions",
            "committed_branch_mispredictions",
            "speculative_branch_mispredictions",
        }
        show_speedup = (
            baseline is not None
            and baseline in config_names
            and (metric in _RATE_METRICS or metric in _LOWER_IS_BETTER)
        )
        if show_speedup and baseline is not None:
            higher_is_better = metric in _RATE_METRICS
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
                speedup_row: List[str] = []
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

        table = Table(labels, config_names, grid, metric, col_header=col_header)
        print(table)
