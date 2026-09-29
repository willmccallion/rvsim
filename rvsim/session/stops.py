"""
Points a :class:`~rvsim.Session` runs to.

A run ends the moment one of its stops holds; ``a | b`` stops at whichever
comes first. Counts are relative to where the run starts. Every run also
ends if the workload does, reported as :class:`Exit`.
"""

from __future__ import annotations

import re
from collections.abc import Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .session import Session

__all__ = [
    "LOGIN_SHELL",
    "AnyOf",
    "Console",
    "Cycles",
    "Exit",
    "Instructions",
    "LoginShell",
    "Marker",
    "Pc",
    "Stop",
    "Stopped",
    "When",
]


class Stop:
    """Where a run stops."""

    def __or__(self, other: Stop) -> AnyOf:
        return AnyOf(self, other)

    def key(self) -> str | None:
        """A stable description of the stop for checkpoint-cache keys, or
        ``None`` when it has none (a predicate, say)."""
        return None

    def _primitives(self) -> tuple[Stop, ...]:
        """The conditions one run checks together."""
        return (self,)

    def _drive(self, session: Session) -> Stopped:
        """Runs ``session`` until this stop holds."""
        return session._run_primitives(self._primitives())


def _count(value: float, what: str) -> int:
    if isinstance(value, float) and value.is_integer():
        value = int(value)
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError(f"{what} takes a positive whole number, got {value!r}")
    return value


@dataclass(frozen=True, init=False)
class Cycles(Stop):
    """After ``count`` more cycles."""

    count: int

    def __init__(self, count: float):
        object.__setattr__(self, "count", _count(count, "Cycles"))

    def key(self) -> str:
        return f"cycles={self.count}"


@dataclass(frozen=True, init=False)
class Instructions(Stop):
    """Once ``count`` more instructions have retired, over all harts."""

    count: int

    def __init__(self, count: float):
        object.__setattr__(self, "count", _count(count, "Instructions"))

    def key(self) -> str:
        return f"instructions={self.count}"


@dataclass(frozen=True, init=False)
class Pc(Stop):
    """When any hart's next instruction to retire is at one of
    ``addresses``."""

    addresses: tuple[int, ...]

    def __init__(self, *addresses: int):
        if not addresses:
            raise ValueError("Pc takes at least one address")
        object.__setattr__(self, "addresses", tuple(addresses))

    def key(self) -> str:
        return "pc=" + ",".join(hex(a) for a in sorted(self.addresses))


@dataclass(frozen=True)
class Marker(Stop):
    """When guest software asks the host to stop: ``rvsim break LABEL`` in
    Linux, ``rvsim_break(label)`` from ``rvsim.h`` on bare metal. Any
    label stops the run when ``label`` is ``None``."""

    label: int | None = None

    def key(self) -> str:
        return f"marker={self.label}"


@dataclass(frozen=True, init=False)
class Console(Stop):
    """When console output the session has not yet matched matches
    ``pattern``, a regular expression; the match consumes the output up to
    its end. The run stops on the cycle the matching output is written."""

    pattern: str
    flags: int

    def __init__(self, pattern: str | re.Pattern[str], flags: int = 0):
        if isinstance(pattern, re.Pattern):
            pattern, flags = pattern.pattern, pattern.flags | flags
        re.compile(pattern, flags)
        object.__setattr__(self, "pattern", pattern)
        object.__setattr__(self, "flags", flags)

    @property
    def regex(self) -> re.Pattern[str]:
        return re.compile(self.pattern, self.flags)

    def key(self) -> str:
        return f"console={self.pattern!r}/{self.flags}"


@dataclass(frozen=True)
class Exit(Stop):
    """When the workload ends."""

    def key(self) -> str:
        return "exit"


@dataclass(frozen=True, eq=False)
class When(Stop):
    """When ``predicate(session)`` returns true, checked every ``every``
    cycles. Give it a ``name`` to let a fast-forward to it be cached; the
    name stands for the predicate, so it must change when the predicate
    does."""

    predicate: Callable[[Session], bool]
    every: int = 100_000
    name: str | None = None

    def __post_init__(self):
        object.__setattr__(self, "every", _count(self.every, "When.every"))

    def key(self) -> str | None:
        return None if self.name is None else f"when={self.name}/{self.every}"


@dataclass(frozen=True, init=False)
class AnyOf(Stop):
    """Whichever of ``stops`` holds first."""

    stops: tuple[Stop, ...]

    def __init__(self, *stops: Stop):
        flat = tuple(p for stop in stops for p in stop._primitives())
        if not flat:
            raise ValueError("AnyOf takes at least one stop")
        object.__setattr__(self, "stops", flat)

    def key(self) -> str | None:
        keys = [stop.key() for stop in self.stops]
        if any(key is None for key in keys):
            return None
        return "any(" + ",".join(sorted(keys)) + ")"

    def _primitives(self) -> tuple[Stop, ...]:
        return self.stops


@dataclass(frozen=True)
class LoginShell(Stop):
    """At a Linux shell: waits for the ``login`` prompt, logs in as
    ``user`` (with ``password`` if the image asks for one), and stops once
    the shell ``prompt`` appears."""

    user: str = "root"
    password: str | None = None
    login: str = r"login: $"
    prompt: str = r"# $"

    def key(self) -> str:
        return f"login_shell(user={self.user!r},login={self.login!r},prompt={self.prompt!r})"

    def _primitives(self) -> tuple[Stop, ...]:
        raise TypeError(
            "LoginShell is a sequence of console steps; it cannot be combined with |"
        )

    def _drive(self, session: Session) -> Stopped:
        for step in self._steps():
            if isinstance(step, str):
                session.send(step)
                continue
            stopped = session._run_primitives((step,))
            if stopped.exited:
                return stopped
        return Stopped(
            by=self,
            cycle=stopped.cycle,
            instructions=stopped.instructions,
            match=stopped.match,
        )

    def _steps(self):
        yield Console(self.login)
        yield self.user + "\n"
        if self.password is not None:
            yield Console(r"[Pp]assword: $")
            yield self.password + "\n"
        yield Console(self.prompt)


#: A root shell on the bundled Linux image.
LOGIN_SHELL = LoginShell()


@dataclass(frozen=True)
class Stopped:
    """Where a run stopped, and which stop ended it."""

    by: Stop
    """The stop that held; an :class:`Exit` when the workload ended."""
    cycle: int
    """The cycle count since the system started."""
    instructions: int
    """Instructions retired by every hart since the system started."""
    exit_code: int | None = None
    """The workload's exit code, when it ended."""
    hart: int | None = None
    """The hart that reached a :class:`Pc` stop."""
    label: int | None = None
    """The guest's label, for a :class:`Marker`."""
    match: re.Match[str] | None = None
    """The console match, for a :class:`Console` or :class:`LoginShell`."""

    @property
    def exited(self) -> bool:
        return isinstance(self.by, Exit)
