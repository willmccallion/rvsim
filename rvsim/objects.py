"""
Simulation objects and high-level run API.

Provides:
- Cpu: Pythonic wrapper with .pc, .regs[i], .mem32[addr], .stats, .run()
- System: Top-level system builder; instantiate() creates the Rust PySystem.
- Simulator: Fluent API (config/kernel/disk/binary/run).
- Instruction: Returned by cpu.step() with pc, raw, asm, cycles.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from typing import Any, Dict, Optional

from ._cli import info, warn, error, tag
from ._core import PySystem, PyCpu
from .config import Config, _config_to_dict
from .stats import Stats


class _RegisterFile:
    """Indexable register access: ``cpu.regs[10]`` reads x10."""

    def __init__(self, rust_cpu: PyCpu):
        self._cpu = rust_cpu

    def __getitem__(self, idx: int) -> int:
        return self._cpu.read_register(idx)

    def __setitem__(self, idx: int, value: int) -> None:
        self._cpu.write_register(idx, value)

    def __repr__(self) -> str:
        vals = [
            f"x{i}={self._cpu.read_register(i)}"
            for i in range(32)
            if self._cpu.read_register(i) != 0
        ]
        return f"RegisterFile({', '.join(vals)})"


class _MemView:
    """Indexable memory access: ``cpu.mem32[addr]`` reads a u32."""

    def __init__(self, rust_cpu: PyCpu, width: int):
        self._cpu = rust_cpu
        self._width = width

    def __getitem__(self, addr: int) -> int:
        if self._width == 32:
            return self._cpu.read_memory_u32(addr)
        elif self._width == 64:
            return self._cpu.read_memory_u64(addr)
        raise ValueError(f"Unsupported width: {self._width}")


class Instruction:
    """Result of a single-step execution."""

    __slots__ = ("pc", "raw", "asm", "cycles")

    def __init__(self, pc: int, raw: int, asm: str, cycles: int):
        self.pc = pc
        self.raw = raw
        self.asm = asm
        self.cycles = cycles

    def __repr__(self) -> str:
        return f"Instruction(pc={self.pc:#x}, asm={self.asm!r}, cycles={self.cycles})"


class Cpu:
    """
    Pythonic CPU wrapper around the Rust PyCpu.

    Properties:
        pc: Program counter (read/write)
        privilege: Current privilege level (read)
        stats: Performance statistics (read, returns Stats)
        trace: Instruction tracing (read/write)
        regs: Register file (_RegisterFile, indexable)
        mem32: Memory view for u32 reads (_MemView)
        mem64: Memory view for u64 reads (_MemView)

    Methods:
        step(): Execute one instruction, return Instruction
        run(): Run until exit with optional limit and progress
        tick(): Advance one cycle
        csr(name): Read a CSR by name or address
        get_pc_trace(): Get committed PC trace
    """

    def __init__(self, rust_cpu: PyCpu):
        self._cpu = rust_cpu
        self._regs = _RegisterFile(rust_cpu)
        self._mem32 = _MemView(rust_cpu, 32)
        self._mem64 = _MemView(rust_cpu, 64)

    @property
    def pc(self) -> int:
        return self._cpu.get_pc()

    @pc.setter
    def pc(self, value: int) -> None:
        self._cpu.set_pc(value)

    @property
    def stats(self) -> Stats:
        raw = self._cpu.get_stats()
        if hasattr(raw, "to_dict"):
            return Stats(raw.to_dict())
        return Stats(dict(raw) if not isinstance(raw, dict) else raw)

    @property
    def regs(self) -> _RegisterFile:
        return self._regs

    @property
    def mem32(self) -> _MemView:
        return self._mem32

    @property
    def mem64(self) -> _MemView:
        return self._mem64

    def step(self) -> Optional[Instruction]:
        """Execute one instruction. Returns Instruction or None if sim exited."""
        result = self._cpu.step_instruction()
        if result is None:
            return None
        pc, raw, asm = result
        cycles = self._cpu.get_stats().cycles
        return Instruction(pc, raw, asm, cycles)

    def run(
        self,
        limit: Optional[int] = None,
        progress: int = 0,
        print_stats: bool = False,
        stats_sections: Optional[list] = None,
    ) -> Optional[int]:
        """Run the simulation until exit or cycle limit.

        Args:
            limit: Max cycles to simulate. ``None`` means unlimited.
            progress: Print progress every N cycles to stderr. 0 = silent.
            print_stats: Print performance stats on completion/error (legacy).
            stats_sections: Sections to print (``[]`` = all, ``None`` = suppress,
                ``["summary", ...]`` = specific). Overrides *print_stats* when set.

        Returns:
            Exit code, or ``None`` if *limit* was reached without exiting.
        """
        raw = self._cpu

        def _stats():
            if stats_sections is not None:
                s = raw.get_stats()
                if stats_sections:
                    s.print_sections(stats_sections)
                else:
                    s.print()
            elif print_stats:
                raw.get_stats().print()

        try:
            if progress > 0:
                cycles_run = 0
                while True:
                    chunk = progress
                    if limit is not None:
                        remaining = limit - cycles_run
                        if remaining <= 0:
                            print(file=sys.stderr)
                            _stats()
                            return None
                        chunk = min(chunk, remaining)
                    exit_code = raw.run(limit=chunk)
                    cycles_run += chunk
                    if exit_code is not None:
                        print(file=sys.stderr)
                        _stats()
                        return int(exit_code)
                    s = raw.get_stats()
                    print(
                        f"\r{tag('rvsim', stderr=True)} {s.cycles:,} cycles, "
                        f"{s.instructions_retired:,} insns",
                        end="",
                        file=sys.stderr,
                        flush=True,
                    )
            else:
                exit_code = raw.run(limit=limit)
                _stats()
                if exit_code is not None:
                    return int(exit_code)
                return None
        except Exception:
            if progress > 0:
                print(file=sys.stderr)
            _stats()
            raise

    def tick(self) -> None:
        """Advance one cycle."""
        self._cpu.tick()

    def csr(self, name) -> int:
        """Read a CSR by name (str) or address (int)."""
        from .isa import csr as _csr_lookup

        addr = _csr_lookup(name) if isinstance(name, str) else name
        return self._cpu.read_csr(addr)

    def get_pc_trace(self):
        """Get the committed PC trace from the pipeline."""
        return self._cpu.get_pc_trace()

    def load_kernel(self, kernel_path: str, dtb_path: Optional[str] = None) -> None:
        """Load a kernel image and optionally a DTB."""
        config_dict = self._config_dict if hasattr(self, "_config_dict") else {}
        self._cpu.load_kernel(kernel_path, config_dict, dtb_path)

    @property
    def raw(self) -> PyCpu:
        """Direct access to underlying Rust CPU for advanced use."""
        return self._cpu


class System:
    """Top-level system builder. Wraps PySystem creation."""

    def __init__(self, ram_size="128MB", ram_base=0x80000000, trace=False):
        from .types import _parse_size

        self.ram_size = _parse_size(ram_size)
        self.ram_base = ram_base
        self.trace = trace
        self.rust_system: Optional[PySystem] = None

    def instantiate(
        self, disk_image=None, config: Optional[Config | Dict[str, Any]] = None
    ) -> PySystem:
        """Create the Rust PySystem from config."""
        if config is not None:
            config_dict = _config_to_dict(config)
        else:
            config_dict = Config(
                ram_size=self.ram_size,
                ram_base=self.ram_base,
                trace=self.trace,
                initial_sp=self.ram_base + 0x100_0000,
            ).to_dict()

        self.rust_system = PySystem(config_dict, disk_image)
        return self.rust_system


def _create_cpu(
    system: System, config: Optional[Config | Dict[str, Any]] = None
) -> Cpu:
    """Internal: create a Cpu from a System and Config."""
    if config is not None:
        config_dict = _config_to_dict(config)
    else:
        config_dict = Config().to_dict()
    rust_cpu = PyCpu(system.rust_system, config_dict)
    cpu = Cpu(rust_cpu)
    cpu._config_dict = config_dict
    return cpu


class Simulator:
    """Fluent API for configuring and running the simulator."""

    def __init__(self):
        self._config_path = None
        self._kernel_path = None
        self._disk_path = None
        self._dtb_path = None
        self._binary_path = None
        self._config_obj: Optional[Config] = None
        self._is_kernel_mode = False

    def with_config(self, config: Config) -> Simulator:
        """Set the machine configuration directly."""
        self._config_obj = config
        return self

    def config(self, path: str) -> Simulator:
        """Load configuration from a Python file."""
        if not os.path.exists(path):
            if os.path.exists(os.path.join(os.getcwd(), path)):
                path = os.path.join(os.getcwd(), path)
            else:
                print(warn(f"Config file {path} not found."), file=sys.stderr)
                return self

        self._config_path = path
        try:
            spec = importlib.util.spec_from_file_location("custom_config", path)
            if spec and spec.loader:
                mod = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(mod)

                name = os.path.splitext(os.path.basename(path))[0]
                if hasattr(mod, name):
                    func = getattr(mod, name)
                    if callable(func):
                        self._config_obj = func()
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
                    self._config_obj = c() if callable(c) else c
                    print(
                        info("Simulator", f"Loaded config from 'config' in {path}"),
                        file=sys.stderr,
                    )
                elif hasattr(mod, "get_config"):
                    self._config_obj = getattr(mod, "get_config")()
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

    def kernel(self, path: str) -> Simulator:
        self._kernel_path = path
        return self

    def disk(self, path: str) -> Simulator:
        self._disk_path = path
        return self

    def dtb(self, path: str) -> Simulator:
        self._dtb_path = path
        return self

    def kernel_mode(self) -> Simulator:
        self._is_kernel_mode = True
        return self

    def binary(self, path: str) -> Simulator:
        self._binary_path = path
        return self

    _UNSET = object()

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
                Defaults to ``[]`` (print all) for backward compatibility.
            output_stats: Path to write JSON stats after simulation.

        Returns:
            Exit code (int).
        """
        if stats_sections is self._UNSET:
            stats_sections = []  # backward compat: print all
        print(info("Simulator", "Setting up...", stderr=True), file=sys.stderr)
        if self._config_obj is None:
            print(
                info("Simulator", "No config loaded, using defaults.", stderr=True),
                file=sys.stderr,
            )
            self._config_obj = Config()
        if self._is_kernel_mode:
            self._config_obj.uart_to_stderr = True

        sys_obj = System(ram_size=self._config_obj.ram_size)
        sys_obj.instantiate(disk_image=self._disk_path, config=self._config_obj)

        # Load binary BEFORE creating CPU, because PyCpu consumes the system.
        # Kernel loading happens after CPU creation via cpu.load_kernel().
        elf_entry = None
        has_tohost = False
        if not self._is_kernel_mode and self._binary_path:
            print(
                info("Simulator", f"Loading ELF: {self._binary_path}", stderr=True),
                file=sys.stderr,
            )
            with open(self._binary_path, "rb") as f:
                data = f.read()
            entry, tohost = sys_obj.rust_system.load_elf(data)
            elf_entry = entry
            if tohost is not None:
                has_tohost = True
                print(
                    info("Simulator", f"ELF tohost @ {tohost:#x}", stderr=True),
                    file=sys.stderr,
                )
        elif not self._is_kernel_mode and not self._kernel_path:
            print(warn("No binary or kernel specified."), file=sys.stderr)

        cpu = _create_cpu(sys_obj, config=self._config_obj)

        # If ELF loading returned an entry point, set the PC.
        if elf_entry is not None:
            cpu.pc = elf_entry

        # riscv-tests ELFs with tohost need M-mode trap handling, not direct mode.
        # Also mark the HTIF range so stores bypass the RAM fast-path.
        if has_tohost:
            cpu.raw.set_direct_mode(False)
            cpu.raw.set_htif_range(tohost, 16)

        if self._is_kernel_mode:
            if not self._kernel_path:
                raise ValueError(
                    "Kernel mode requested but no kernel path provided via .kernel()"
                )
            print(
                info("Simulator", f"Loading kernel: {self._kernel_path}", stderr=True),
                file=sys.stderr,
            )
            if self._dtb_path:
                print(
                    info("Simulator", f"Loading DTB: {self._dtb_path}", stderr=True),
                    file=sys.stderr,
                )
            cpu.load_kernel(self._kernel_path, self._dtb_path)

        exit_code = cpu.run(
            limit=limit, progress=progress, stats_sections=stats_sections
        )

        # Write JSON stats if requested
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
