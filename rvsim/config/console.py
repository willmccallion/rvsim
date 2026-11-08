"""Where the guest's console connects."""

from __future__ import annotations

from enum import Enum

__all__ = ["ConsoleMode"]


class ConsoleMode(str, Enum):
    """Where the guest's console connects: the UART, HTIF's console and a
    user program's stdin, stdout and stderr. Equal to its value as a
    string, so ``Config(console="Quiet")`` names the same mode."""

    STDOUT = "Stdout"
    """The host's stdout, with input from its stdin."""
    STDERR = "Stderr"
    """The host's stderr, with input from its stdin."""
    QUIET = "Quiet"
    """Output discarded; no input."""
    CAPTURED = "Captured"
    """Output kept for ``read_console()``; input given by ``write_console()``."""

    def __str__(self) -> str:
        return self.value
