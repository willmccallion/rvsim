"""
The simulator.

- :class:`Simulator`: the native simulator. Accepts a :class:`Config` (or
  config dict) plus optional ``binary`` / ``kernel`` / ``disk`` / ``dtb`` paths.
- :class:`Instruction`: returned by ``sim.step()`` with pc, raw, asm, cycles.
- :class:`PipelineSnapshot`: returned by ``sim.pipeline_snapshot()``; its
  ``.render()`` / ``.visualize()`` draw every inter-stage latch as a
  Gantt-style diagram.
"""

from __future__ import annotations

__all__ = ["Instruction", "PipelineSnapshot", "Simulator"]

from typing import TYPE_CHECKING

from ._core import Instruction, PipelineSnapshot
from ._core import Simulator as _CoreSimulator
from .config import Config
from .config._config import _config_to_dict

if TYPE_CHECKING:
    from typing_extensions import Self


class Simulator(_CoreSimulator):
    """Native simulator.

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
        kernel: str | None = None,
        firmware: str | None = None,
        disk: str | None = None,
        dtb: str | None = None,
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

        return _CoreSimulator.__new__(
            cls,
            config_dict,
            elf_data=elf_data,
            kernel_path=kernel,
            firmware_path=firmware,
            dtb_path=dtb,
            disk_path=disk,
        )
