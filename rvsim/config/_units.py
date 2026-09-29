"""Parsing human-readable sizes ("32KB") and cycle counts ("10M")."""

from __future__ import annotations


import re


def _parse_size(s) -> int:
    """Parse a size string like '32KB', '4MB', '64B' into bytes. Ints pass through."""
    if isinstance(s, int):
        return s
    if not isinstance(s, str):
        raise TypeError(f"Expected str or int, got {type(s).__name__}")
    m = re.fullmatch(r"(\d+)\s*(B|KB|MB|GB)", s.strip(), re.IGNORECASE)
    if not m:
        raise ValueError(f"Cannot parse size: {s!r} (expected e.g. '32KB', '4MB')")
    val = int(m.group(1))
    unit = m.group(2).upper()
    mult = {"B": 1, "KB": 1024, "MB": 1024**2, "GB": 1024**3}
    return val * mult[unit]


def _parse_cycles(s) -> int:
    """Parse a cycle count like ``'5M'``, ``'500K'``, ``'2G'`` into an integer.

    Uses SI (decimal) suffixes: K = 1,000, M = 1,000,000, G = 1,000,000,000.
    Plain integers and numeric strings pass through.
    """
    if isinstance(s, int):
        return s
    if not isinstance(s, str):
        raise TypeError(f"Expected str or int, got {type(s).__name__}")
    m = re.fullmatch(r"(\d+)\s*([KMG])?", s.strip(), re.IGNORECASE)
    if not m:
        raise ValueError(
            f"Cannot parse cycle count: {s!r} (expected e.g. '5M', '500K', or plain integer)"
        )
    val = int(m.group(1))
    suffix = (m.group(2) or "").upper()
    mult = {"": 1, "K": 1_000, "M": 1_000_000, "G": 1_000_000_000}
    return val * mult[suffix]
