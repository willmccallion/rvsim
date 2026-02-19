"""
Simulation objects and high-level run API.

Provides:
- Cpu: Pythonic wrapper with .pc, .regs[i], .mem32[addr], .stats, .step()
- System: Top-level system builder; instantiate() creates the Rust PySystem.
- Simulator: Fluent API (config/kernel/disk/binary/run).
- Instruction: Returned by cpu.step() with pc, raw, asm, cycles.
- simulate / run_with_progress: Run helpers.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from typing import Any, Dict, Optional

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
        run(limit): Run until exit or cycle limit
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

    def run(self, limit: Optional[int] = None) -> Optional[int]:
        """Run until exit or cycle limit. Returns exit code or None."""
        return self._cpu.run(limit=limit)

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
        # Need config dict for kernel loading — we stored it during creation
        config_dict = self._config_dict if hasattr(self, "_config_dict") else {}
        self._cpu.load_kernel(kernel_path, config_dict, dtb_path)

    # Direct access to underlying Rust CPU for advanced use
    @property
    def raw(self) -> PyCpu:
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


def run_with_progress(
    cpu, progress_interval_cycles: int = 5_000_000, progress_stream=None
):
    """Run the CPU in chunks, printing cycle progress. Returns exit code."""
    if progress_stream is None:
        progress_stream = sys.stderr
    # Accept either Cpu wrapper or raw PyCpu
    raw = cpu._cpu if isinstance(cpu, Cpu) else cpu
    while True:
        exit_code = raw.run(limit=progress_interval_cycles)
        if exit_code is not None:
            return int(exit_code)
        sys.stdout.flush()


def simulate(cpu, print_stats=True, stats_sections=None):
    """Run the CPU until exit. Optionally print stats."""
    print("Starting simulation...")
    sys.stdout.flush()
    # Accept either Cpu wrapper or raw PyCpu
    raw = cpu._cpu if isinstance(cpu, Cpu) else cpu
    try:
        exit_code = raw.run()
        if exit_code is None:
            raise RuntimeError(
                "CPU run completed without exit code (should not happen without limit)"
            )
        if print_stats:
            stats = raw.get_stats()
            if stats_sections is not None:
                stats.print_sections(stats_sections)
            else:
                stats.print()
        return exit_code
    except Exception as e:
        print(f"Simulation stopped: {e}")
        if print_stats:
            stats = raw.get_stats()
            if stats_sections is not None:
                stats.print_sections(stats_sections)
            else:
                stats.print()
        raise


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
                print(f"Warning: Config file {path} not found.")
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
                        print(f"[Simulator] Loaded config from {name}() in {path}")
                    else:
                        print(
                            f"[Simulator] Found {name} in {path} but it is not callable."
                        )
                elif hasattr(mod, "config"):
                    c = getattr(mod, "config")
                    self._config_obj = c() if callable(c) else c
                    print(f"[Simulator] Loaded config from 'config' in {path}")
                elif hasattr(mod, "get_config"):
                    self._config_obj = getattr(mod, "get_config")()
                    print(f"[Simulator] Loaded config from get_config() in {path}")
                else:
                    print(
                        f"[Simulator] Could not find config entry point in {path}. "
                        f"Expected function '{name}' or 'get_config' or variable 'config'."
                    )
        except Exception as e:
            print(f"Error loading config {path}: {e}")

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

    def run(self) -> int:
        """Build system and CPU from config, load binary or kernel, then run. Returns exit code."""
        print("[Simulator] Setting up...")
        if self._config_obj is None:
            print("[Simulator] No config loaded, using defaults.")
            self._config_obj = Config()
        if self._is_kernel_mode:
            self._config_obj.uart_to_stderr = True

        sys_obj = System(ram_size=self._config_obj.ram_size)
        sys_obj.instantiate(disk_image=self._disk_path, config=self._config_obj)

        # Load binary BEFORE creating CPU, because PyCpu consumes the system.
        # Kernel loading happens after CPU creation via cpu.load_kernel().
        if not self._is_kernel_mode and self._binary_path:
            print(f"[Simulator] Loading binary: {self._binary_path}")
            with open(self._binary_path, "rb") as f:
                sys_obj.rust_system.load_binary(f.read(), 0x80000000)
        elif not self._is_kernel_mode and not self._kernel_path:
            print("Warning: No binary or kernel specified.")

        cpu = _create_cpu(sys_obj, config=self._config_obj)

        if self._is_kernel_mode:
            if not self._kernel_path:
                raise ValueError(
                    "Kernel mode requested but no kernel path provided via .kernel()"
                )
            print(f"[Simulator] Loading kernel: {self._kernel_path}")
            if self._dtb_path:
                print(f"[Simulator] Loading DTB: {self._dtb_path}")
            cpu.load_kernel(self._kernel_path, self._dtb_path)

        if self._is_kernel_mode:
            print("Starting simulation (progress every 5M cycles; UART = stderr)...")
            sys.stdout.flush()
            try:
                exit_code = run_with_progress(cpu, progress_interval_cycles=5_000_000)
                print(file=sys.stderr)
                raw_stats = cpu.raw.get_stats()
                raw_stats.print()
                return exit_code
            except Exception as e:
                print(file=sys.stderr)
                print(f"Simulation stopped: {e}")
                cpu.raw.get_stats().print()
                raise
        return simulate(cpu.raw)
