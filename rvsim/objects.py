"""
Simulation objects and high-level run API.

Provides:
- Cpu: Native Rust CPU class with .pc, .regs[i], .csrs[name], .mem32[addr], .stats, .run()
- Simulator: Fluent API (config/kernel/disk/binary/run).
- Instruction: Returned by cpu.step() with pc, raw, asm, cycles.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from typing import Optional

__all__ = ["Cpu", "Simulator", "Instruction"]

from ._cli import info, warn, error
from ._core import Cpu, Instruction
from .config import Config, _config_to_dict

_UNSET = object()


class Simulator:
    """Fluent API for configuring and running the simulator.

    Example::

        cpu = (
            Simulator()
            .config(Config(width=4))
            .binary("qsort.elf")
            .build()
        )
        exit_code = cpu.run(limit=10_000_000)
    """

    def __init__(self):
        self._kernel_path = None
        self._disk_path = None
        self._dtb_path = None
        self._binary_path = None
        self._config_obj: Optional[Config] = None

    def config(self, path_or_config) -> "Simulator":
        """Set the machine configuration.

        Accepts either a :class:`Config` object or a path to a Python config file.
        When given a file path, the module is imported and the first of these is used:
        a function named after the file, a ``config`` variable, or a ``get_config``
        function.

        Args:
            path_or_config: A :class:`Config` instance or a ``str`` path to a config file.
        """
        if isinstance(path_or_config, Config):
            self._config_obj = path_or_config
            return self

        path = path_or_config
        if not os.path.exists(path):
            if os.path.exists(os.path.join(os.getcwd(), path)):
                path = os.path.join(os.getcwd(), path)
            else:
                print(warn(f"Config file {path} not found."), file=sys.stderr)
                return self

        try:
            spec = importlib.util.spec_from_file_location("custom_config", path)
            if spec and spec.loader:
                mod = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(mod)

                name = os.path.splitext(os.path.basename(path))[0]
                if hasattr(mod, name):
                    func = getattr(mod, name)
                    if callable(func):
                        self._config_obj = func()  # type: ignore[assignment]
                        print(
                            info("Simulator", f"Loaded config from {name}() in {path}"),
                            file=sys.stderr,
                        )
                    else:
                        print(
                            warn(f"Found {name} in {path} but it is not callable."),
                            file=sys.stderr,
                        )
                elif hasattr(mod, "config"):
                    c = getattr(mod, "config")
                    self._config_obj = c() if callable(c) else c  # type: ignore[assignment]
                    print(
                        info("Simulator", f"Loaded config from 'config' in {path}"),
                        file=sys.stderr,
                    )
                elif hasattr(mod, "get_config"):
                    self._config_obj = getattr(mod, "get_config")()  # type: ignore[assignment]
                    print(
                        info("Simulator", f"Loaded config from get_config() in {path}"),
                        file=sys.stderr,
                    )
                else:
                    print(
                        warn(
                            f"Could not find config entry point in {path}. "
                            f"Expected function '{name}' or 'get_config' or variable 'config'."
                        ),
                        file=sys.stderr,
                    )
        except Exception as e:
            print(error(f"Loading config {path}: {e}"), file=sys.stderr)

        return self

    def kernel(self, path: str) -> "Simulator":
        """Set the kernel image path. Enables kernel mode automatically."""
        self._kernel_path = path
        return self

    def disk(self, path: str) -> "Simulator":
        self._disk_path = path
        return self

    def dtb(self, path: str) -> "Simulator":
        self._dtb_path = path
        return self

    def binary(self, path: str) -> "Simulator":
        self._binary_path = path
        return self

    def build(self) -> Cpu:
        """Build system and CPU from config, load binary or kernel, and return the Cpu.

        Returns:
            A configured :class:`Cpu` instance ready for ``run()``, ``step()``, etc.
        """
        print(info("Simulator", "Setting up...", stderr=True), file=sys.stderr)
        if self._config_obj is None:
            print(
                info("Simulator", "No config loaded, using defaults.", stderr=True),
                file=sys.stderr,
            )
            self._config_obj = Config()

        is_kernel_mode = self._kernel_path is not None
        if is_kernel_mode:
            self._config_obj.uart_to_stderr = True

        config_dict = _config_to_dict(self._config_obj)

        # Read ELF data if in bare-metal mode
        elf_data = None
        if not is_kernel_mode and self._binary_path:
            print(
                info("Simulator", f"Loading ELF: {self._binary_path}", stderr=True),
                file=sys.stderr,
            )
            with open(self._binary_path, "rb") as f:
                elf_data = f.read()
        elif not is_kernel_mode and not self._binary_path:
            print(warn("No binary or kernel specified."), file=sys.stderr)

        kernel_path = None
        dtb_path = None
        if is_kernel_mode:
            print(
                info("Simulator", f"Loading kernel: {self._kernel_path}", stderr=True),
                file=sys.stderr,
            )
            if self._dtb_path:
                print(
                    info("Simulator", f"Loading DTB: {self._dtb_path}", stderr=True),
                    file=sys.stderr,
                )
            kernel_path = self._kernel_path
            dtb_path = self._dtb_path

        cpu = Cpu(
            config_dict,
            elf_data=elf_data,
            kernel_path=kernel_path,
            dtb_path=dtb_path,
            disk_path=self._disk_path,
        )

        return cpu

    def run(
        self,
        limit: Optional[int] = None,
        progress: int = 0,
        stats_sections=_UNSET,
        output_stats: Optional[str] = None,
    ) -> int:
        """Build system and CPU from config, load binary or kernel, then run.

        Args:
            limit: Maximum number of cycles. ``None`` means unlimited.
            progress: Print progress every N cycles. 0 = silent.
            stats_sections: Stats sections to print (``[]`` = all, ``None`` = suppress).
                Defaults to ``[]`` (print all).
            output_stats: Path to write JSON stats after simulation.

        Returns:
            Exit code (int).
        """
        resolved_sections: Optional[list] = (
            [] if stats_sections is _UNSET else stats_sections
        )  # type: ignore[assignment]

        cpu = self.build()

        exit_code = cpu.run(
            limit=limit, progress=progress, stats_sections=resolved_sections
        )

        if output_stats is not None:
            import json

            stats_dict = dict(cpu.stats)
            with open(output_stats, "w") as f:
                json.dump(stats_dict, f, indent=2)
            print(
                info("rvsim", f"Stats written to {output_stats}", stderr=True),
                file=sys.stderr,
            )

        if exit_code is None:
            print(
                warn(f"Simulation did not exit within {limit:,} cycles."),
                file=sys.stderr,
            )
            return 1

        print(
            info("rvsim", f"Exited with code {exit_code}", stderr=True),
            file=sys.stderr,
        )
        return exit_code
