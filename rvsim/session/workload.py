"""What a session runs, and the digests that identify it in cache keys."""

from __future__ import annotations

import hashlib
import json
import os
from dataclasses import dataclass
from typing import Any, Dict, Optional, Tuple

from ..simulator import Simulator
from .stops import Stopped


_file_digests: Dict[Tuple[str, int, int], str] = {}


def _digest_json(value: Any) -> str:
    text = json.dumps(value, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(text.encode()).hexdigest()


def _file_digest(path: str) -> str:
    """The file's SHA-256, remembered while its size and mtime hold."""
    stat = os.stat(path)
    identity = (path, stat.st_size, stat.st_mtime_ns)
    if identity not in _file_digests:
        digest = hashlib.sha256()
        with open(path, "rb") as f:
            for block in iter(lambda: f.read(1 << 20), b""):
                digest.update(block)
        _file_digests[identity] = digest.hexdigest()
    return _file_digests[identity]


@dataclass(frozen=True)
class Workload:
    """The software a session runs: a bare-metal ELF (``binary``), or a
    kernel boot (``kernel`` through OpenSBI ``firmware``, with an optional
    ``disk`` and ``dtb``)."""

    binary: Optional[str] = None
    kernel: Optional[str] = None
    firmware: Optional[str] = None
    disk: Optional[str] = None
    dtb: Optional[str] = None

    def __post_init__(self):
        if (self.binary is None) == (self.kernel is None):
            raise ValueError("a workload is a binary or a kernel, exactly one of them")
        if self.binary is not None and (self.firmware or self.dtb):
            raise ValueError("firmware and dtb belong to kernel workloads")
        if self.kernel is not None and self.firmware is None:
            beside = os.path.join(os.path.dirname(self.kernel), "fw_jump.bin")
            if not os.path.exists(beside):
                raise FileNotFoundError(
                    f"no firmware given and no fw_jump.bin beside {self.kernel}; pass firmware="
                )
            object.__setattr__(self, "firmware", beside)
        for role, path in self.files().items():
            if not os.path.isfile(path):
                raise FileNotFoundError(f"{role} not found: {path}")
            object.__setattr__(self, role, os.path.abspath(path))

    @property
    def is_kernel(self) -> bool:
        return self.kernel is not None

    def files(self) -> Dict[str, str]:
        """Each file the workload loads, by role."""
        roles = ("binary", "kernel", "firmware", "disk", "dtb")
        return {
            role: getattr(self, role)
            for role in roles
            if getattr(self, role) is not None
        }

    def digests(self) -> Dict[str, str]:
        """Each file's SHA-256, by role."""
        return {role: _file_digest(path) for role, path in self.files().items()}

    def build(self, config: Dict[str, Any]) -> Simulator:
        return Simulator(
            config,
            binary=self.binary,
            kernel=self.kernel,
            firmware=self.firmware,
            disk=self.disk,
            dtb=self.dtb,
        )


@dataclass
class Region:
    """The stats and console output of one measured region."""

    name: str
    stats: Any
    """The region's stats (a ``Stats``): only what happened inside it."""
    console: str
    """What the guest printed during the region."""
    exit_code: Optional[int]
    """The measured command's exit status, or the workload's exit code."""
    stopped: Optional[Stopped]
    """Where a region measured up to a stop ended."""
    host_seconds: float

    @property
    def cycles(self) -> int:
        return self.stats.cycles

    @property
    def instructions(self) -> int:
        return self.stats.instructions_retired

    @property
    def ipc(self) -> float:
        return self.stats.ipc

    def to_dict(self) -> Dict[str, Any]:
        """A JSON-serializable record of the region, every stat included."""
        return {
            "name": self.name,
            "exit_code": self.exit_code,
            "cycles": self.cycles,
            "instructions": self.instructions,
            "ipc": self.ipc,
            "host_seconds": self.host_seconds,
            "console": self.console,
            "stats": dict(self.stats.query("**")),
        }
