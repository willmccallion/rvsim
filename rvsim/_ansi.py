"""ANSI escape codes for the text the package prints, and whether stdout
shows them."""

import sys

RESET = "\033[0m"
BOLD = "\033[1m"
DIM = "\033[2m"
TEAL = "\033[36m"
PURPLE = "\033[35m"


def stdout_is_tty() -> bool:
    """True when stdout is a terminal, so escape codes render. ``sys.stdout``
    may be ``None`` or a stream without ``isatty`` when Python is embedded."""
    return hasattr(sys.stdout, "isatty") and sys.stdout.isatty()
