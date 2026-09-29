"""ANSI color helpers for CLI output."""

import sys

RESET = "\033[0m"
BOLD = "\033[1m"
TEAL = "\033[36m"
YELLOW = "\033[33m"
RED = "\033[31m"
DIM = "\033[2m"

_color_stderr = hasattr(sys.stderr, "isatty") and sys.stderr.isatty()
_color_stdout = hasattr(sys.stdout, "isatty") and sys.stdout.isatty()


def _c(code: str, text: str, *, stderr: bool = False) -> str:
    if stderr and not _color_stderr:
        return text
    if not stderr and not _color_stdout:
        return text
    return f"{code}{text}{RESET}"


def tag(label: str, *, stderr: bool = False) -> str:
    """Bold teal tag, e.g. ``[rvsim]``."""
    return _c(f"{BOLD}{TEAL}", f"[{label}]", stderr=stderr)


def warn(msg: str) -> str:
    """Yellow ``[WARN] msg`` for stderr."""
    return _c(f"{BOLD}{YELLOW}", "[WARN]", stderr=True) + " " + msg


def error(msg: str) -> str:
    """Red ``[ERROR] msg`` for stderr."""
    return _c(f"{BOLD}{RED}", "[ERROR]", stderr=True) + " " + msg


def info(label: str, msg: str, *, stderr: bool = False) -> str:
    """Tagged info line: ``[label] msg``."""
    return tag(label, stderr=stderr) + " " + msg


def dim(text: str, *, stderr: bool = False) -> str:
    """Dimmed text."""
    return _c(DIM, text, stderr=stderr)
