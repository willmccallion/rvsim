"""
Simulation objects.

Provides:
- Simulator: Native Rust simulator. Accepts a :class:`Config` (or config dict)
  plus optional ``binary`` / ``kernel`` / ``disk`` / ``dtb`` paths.
- Instruction: Returned by ``sim.step()`` with pc, raw, asm, cycles.
"""

from __future__ import annotations

from typing import Optional, Union

__all__ = ["Simulator", "Instruction"]

from ._core import Instruction
from ._core import Simulator as _CoreSimulator
from .config import Config, _config_to_dict


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
        config: Union[Config, dict, None] = None,
        *,
        binary: Optional[str] = None,
        elf_data: Optional[bytes] = None,
        kernel: Optional[str] = None,
        disk: Optional[str] = None,
        dtb: Optional[str] = None,
    ) -> "Simulator":
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
            dtb_path=dtb,
            disk_path=disk,
        )
