"""
The simulator.

- `Simulator`: the native simulator. Accepts a `Config` (or
  config dict) plus optional ``binary`` / ``kernel`` / ``disk`` / ``dtb`` paths.
- `Instruction`: returned by ``sim.step()`` with pc, raw, asm, cycles.
- `PipelineSnapshot`: returned by ``sim.pipeline_snapshot()``; its
  ``.render()`` / ``.visualize()`` draw every inter-stage latch as a
  Gantt-style diagram.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

__all__ = ["Instruction", "PipelineSnapshot", "Program", "Simulator"]

from typing import TYPE_CHECKING

from ._core import Instruction, PipelineSnapshot
from ._core import Simulator as _CoreSimulator
from .config import Config
from .config._config import _config_to_dict

if TYPE_CHECKING:
    from typing_extensions import Self


@dataclass(frozen=True)
class Program:
    """A static Linux user program for SE mode: its binary, its command
    line after its name, and its environment as ``NAME=value`` entries."""

    binary: str
    args: Sequence[str] = ()
    env: Sequence[str] = ()

    def _loaded(self) -> tuple[bytes, list[str], list[str]]:
        with open(self.binary, "rb") as f:
            data = f.read()
        return data, [self.binary, *self.args], list(self.env)


class Simulator(_CoreSimulator):
    """Native simulator.

    ``programs`` runs several static Linux user programs under SE mode at
    once, one per hart from hart 0 up, sharing the memory system, as gem5's
    ``se.py -n N --cmd "a;b"`` does; the run ends when the last of them
    exits, with the first non-zero exit code in hart order, and
    ``exit_codes`` holds each hart's.

    ``script`` names the file the guest's readfile pseudo-operation reads
    and ``outdir`` the directory its writefile pseudo-operation writes
    into, as gem5's ``--script`` and ``m5out``.

    Example::

        sim = Simulator(Config(width=4), binary="qsort.elf")
        exit_code = sim.run(limit=10_000_000)

        sim = Simulator(kernel_config(), kernel="linux.img", disk="rootfs.img")
        sim.run()
    """

    def __new__(
        cls,
        config: Config | dict | None = None,
        *,
        binary: str | None = None,
        elf_data: bytes | None = None,
        args: list[str] | None = None,
        env: list[str] | None = None,
        mode: str | None = None,
        programs: Sequence[Program] | None = None,
        kernel: str | None = None,
        firmware: str | None = None,
        disk: str | None = None,
        dtb: str | None = None,
        script: str | None = None,
        outdir: str | None = None,
    ) -> Self:
        if config is None:
            config_dict = _config_to_dict(Config())
        elif isinstance(config, Config):
            config_dict = _config_to_dict(config)
        else:
            config_dict = config

        if elf_data is None and binary is not None:
            with open(binary, "rb") as f:
                elf_data = f.read()

        if args is None and binary is not None:
            args = [binary]
        loaded = [program._loaded() for program in programs] if programs else None
        return _CoreSimulator.__new__(
            cls,
            config_dict,
            elf_data=elf_data,
            args=args,
            env=env,
            mode=mode,
            programs=loaded,
            kernel_path=kernel,
            firmware_path=firmware,
            dtb_path=dtb,
            disk_path=disk,
            script_path=script,
            outdir=outdir,
        )
