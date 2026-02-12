"""
Simulation objects and high-level run API.

This module provides:
1. **SimObject:** Base for configurable objects with to_dict() for the Rust backend.
2. **System:** Top-level system (RAM size/base, trace); instantiate() creates the Rust PySystem.
3. **P550Cpu:** CPU wrapper with config and branch predictor; create() builds PyCpu, load_kernel() for OS boot.
4. **simulate:** Run CPU until exit with optional stats printing and section filter.
5. **run_with_progress:** Run in chunks with cycle progress (e.g., for kernel boot).
6. **Simulator:** Fluent API (config/kernel/disk/binary/run) for script-based runs.
7. **get_default_config:** Base config dict when no script config is provided.
"""

from __future__ import annotations

import importlib.util
import os
import sys
from typing import Any, Dict, Optional

import riscv_emulator

from .config import SimConfig, config_to_dict


class SimObject:
    """Base class for all simulation objects; holds params and optional children for nested config."""

    def __init__(self, **kwargs):
        self._params = kwargs
        self._children = []

    def add_child(self, child):
        """Append a child SimObject and return it for chaining."""
        self._children.append(child)
        return child

    def to_dict(self):
        """Recursively convert params (and nested SimObjects) to a dict for the Rust backend."""
        d = {}
        for k, v in self._params.items():
            if isinstance(v, SimObject):
                d[k] = v.to_dict()
            else:
                d[k] = v
        return d


class System(SimObject):
    """Top-level system with RAM size/base and trace; instantiate() creates the Rust PySystem."""

    def __init__(self, ram_size="128MB", ram_base=0x80000000, trace=False, **kwargs):
        if isinstance(ram_size, str):
            if "MB" in ram_size:
                ram_size = int(ram_size.replace("MB", "")) * 1024 * 1024
            elif "GB" in ram_size:
                ram_size = int(ram_size.replace("GB", "")) * 1024 * 1024 * 1024

        super().__init__(ram_size=ram_size, ram_base=ram_base, trace=trace, **kwargs)
        self.cpu = None
        self.memory = None
        self.rust_system = None

    def instantiate(
        self, disk_image=None, config: Optional[SimConfig | Dict[str, Any]] = None
    ):
        """Create the Rust System. Pass config= from your script (e.g. scripts/p550/config.py); else uses base."""
        if config is not None:
            config_dict = config_to_dict(config)
        else:
            ram_base = self._params["ram_base"]
            ram_size = self._params["ram_size"]
            config_dict = {
                "general": {
                    "trace_instructions": self._params["trace"],
                    "start_pc": ram_base,
                    "direct_mode": True,
                    "initial_sp": ram_base + 0x100_0000,
                },
                "system": {
                    "ram_base": self._params["ram_base"],
                    "uart_base": 0x10000000,
                    "disk_base": 0x90000000,
                    "clint_base": 0x02000000,
                    "syscon_base": 0x00100000,
                    "kernel_offset": 0x00200000,
                    "bus_width": 8,
                    "bus_latency": 4,
                    "clint_divider": 10,
                },
                "memory": {
                    "ram_size": ram_size,
                    "controller": "Simple",
                    "t_cas": 14,
                    "t_ras": 14,
                    "t_pre": 14,
                    "row_miss_latency": 120,
                    "tlb_size": 32,
                },
                "cache": {
                    "l1_i": {
                        "enabled": False,
                        "size_bytes": 4096,
                        "line_bytes": 64,
                        "ways": 1,
                        "policy": "LRU",
                        "latency": 1,
                        "prefetcher": "None",
                        "prefetch_table_size": 0,
                        "prefetch_degree": 0,
                    },
                    "l1_d": {
                        "enabled": False,
                        "size_bytes": 4096,
                        "line_bytes": 64,
                        "ways": 1,
                        "policy": "LRU",
                        "latency": 1,
                        "prefetcher": "None",
                        "prefetch_table_size": 0,
                        "prefetch_degree": 0,
                    },
                    "l2": {
                        "enabled": False,
                        "size_bytes": 0,
                        "line_bytes": 0,
                        "ways": 0,
                        "policy": "LRU",
                        "latency": 0,
                        "prefetcher": "None",
                        "prefetch_table_size": 0,
                        "prefetch_degree": 0,
                    },
                    "l3": {
                        "enabled": False,
                        "size_bytes": 0,
                        "line_bytes": 0,
                        "ways": 0,
                        "policy": "LRU",
                        "latency": 0,
                        "prefetcher": "None",
                        "prefetch_table_size": 0,
                        "prefetch_degree": 0,
                    },
                },
                "pipeline": {
                    "width": 1,
                    "branch_predictor": "Static",
                    "btb_size": 256,
                    "ras_size": 8,
                    "tage": {
                        "num_banks": 4,
                        "table_size": 2048,
                        "loop_table_size": 256,
                        "reset_interval": 2000,
                        "history_lengths": [],
                        "tag_widths": [],
                    },
                    "perceptron": {"history_length": 32, "table_bits": 10},
                    "tournament": {
                        "global_size_bits": 12,
                        "local_hist_bits": 10,
                        "local_pred_bits": 10,
                    },
                },
            }

        self.rust_system = riscv_emulator.PySystem(config_dict, disk_image)
        return self.rust_system


class P550Cpu(SimObject):
    """CPU wrapper; pass config= from your script (e.g. scripts/p550/config.py) to define the machine."""

    def __init__(
        self,
        system,
        trace=False,
        branch_predictor="TAGE",
        config: Optional[SimConfig | Dict[str, Any]] = None,
        **kwargs,
    ):
        super().__init__(**kwargs)
        self.system = system
        self.trace = trace
        self.branch_predictor = branch_predictor
        self._config = config
        self.rust_cpu = None

    def create(self):
        """Build the Rust PyCpu from the current system and config; returns the Rust CPU."""
        config_dict = self._get_config_dict()
        self.rust_cpu = riscv_emulator.PyCpu(self.system.rust_system, config_dict)
        return self.rust_cpu

    def _get_config_dict(self):
        if self._config is not None:
            config_dict = config_to_dict(self._config)
            config_dict["general"]["trace_instructions"] = self.trace
            config_dict["pipeline"]["branch_predictor"] = self.branch_predictor
            return config_dict
        else:
            cfg = SimConfig.default()
            cfg.general.trace_instructions = self.trace
            cfg.pipeline.width = 1
            cfg.pipeline.branch_predictor = self.branch_predictor
            return cfg.to_dict()

    def load_kernel(self, kernel_path: str, dtb_path: Optional[str] = None):
        """Load a kernel image and optionally a DTB. Enables kernel mode."""
        if self.rust_cpu is None:
            raise RuntimeError("CPU not created yet. Call create() first.")

        config_dict = self._get_config_dict()
        self.rust_cpu.load_kernel(kernel_path, config_dict, dtb_path)


def get_default_config() -> Dict[str, Any]:
    """Return the base config (1-wide, static BP, caches off). Build your machine in scripts—see scripts/p550/config.py, scripts/m1/config.py."""
    return SimConfig.default().to_dict()


def run_with_progress(
    cpu, progress_interval_cycles: int = 5_000_000, progress_stream=None
):
    """Run the CPU in chunks, printing cycle progress so kernel boot doesn't appear to hang.
    Progress is written to progress_stream (default stderr). Stdout is flushed so UART output appears.
    Returns exit code when the program exits.
    """
    if progress_stream is None:
        progress_stream = sys.stderr
    while True:
        exit_code = cpu.run_with_limit(progress_interval_cycles)
        if exit_code is not None:
            return int(exit_code)
        cycles = cpu.get_stats().cycles
        # print(f"\r[sim] cycles: {cycles:,}  ", end="", file=progress_stream, flush=True)
        sys.stdout.flush()


def simulate(cpu, print_stats=True, stats_sections=None):
    """Run the CPU until the program exits.
    If print_stats is True (default), print stats after run (gem5-style).
    stats_sections: optional list of section names to print. If None, prints full dump.
    Sections: "summary", "core", "instruction_mix", "branch", "memory".
    Example: stats_sections=["summary", "memory"] for cycles + cache stats only (e.g. multisim sweep).
    """
    print("Starting simulation...")
    sys.stdout.flush()
    try:
        exit_code = cpu.run()
        if print_stats:
            stats = cpu.get_stats()
            if stats_sections is not None:
                stats.print_sections(stats_sections)
            else:
                stats.print()
        return exit_code
    except Exception as e:
        print(f"Simulation stopped: {e}")
        if print_stats:
            stats = cpu.get_stats()
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
        self._config_obj = None
        self._is_kernel_mode = False

    def config(self, path: str) -> Simulator:
        """Load configuration from a Python file (looks for function named like the file or get_config/config)."""
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
                        f"[Simulator] Could not find config entry point in {path}. Expected function '{name}' or 'get_config' or variable 'config'."
                    )
        except Exception as e:
            print(f"Error loading config {path}: {e}")

        return self

    def kernel(self, path: str) -> Simulator:
        """Set kernel image path for OS boot."""
        self._kernel_path = path
        return self

    def disk(self, path: str) -> Simulator:
        """Set disk image path."""
        self._disk_path = path
        return self

    def dtb(self, path: str) -> Simulator:
        """Set device tree blob path."""
        self._dtb_path = path
        return self

    def kernel_mode(self) -> Simulator:
        """Enable kernel boot (load kernel, UART to stderr, progress updates)."""
        self._is_kernel_mode = True
        return self

    def binary(self, path: str) -> Simulator:
        """Set bare-metal binary path for direct execution."""
        self._binary_path = path
        return self

    def run(self) -> int:
        """Build system and CPU from config, load binary or kernel, then run until exit. Returns exit code."""
        print("[Simulator] Setting up...")
        if self._config_obj is None:
            print("[Simulator] No config loaded, using defaults.")
            self._config_obj = SimConfig.default()
        if self._is_kernel_mode:
            self._config_obj.system.uart_to_stderr = True
        ram_size = self._config_obj.memory.ram_size

        sys_obj = System(ram_size=ram_size)
        sys_obj.instantiate(disk_image=self._disk_path, config=self._config_obj)

        cpu_obj = P550Cpu(sys_obj, config=self._config_obj)
        rust_cpu = cpu_obj.create()

        if self._is_kernel_mode:
            if not self._kernel_path:
                raise ValueError(
                    "Kernel mode requested but no kernel path provided via .kernel()"
                )
            print(f"[Simulator] Loading kernel: {self._kernel_path}")
            if self._dtb_path:
                print(f"[Simulator] Loading DTB: {self._dtb_path}")
            cpu_obj.load_kernel(self._kernel_path, self._dtb_path)
        elif self._binary_path:
            print(f"[Simulator] Loading binary: {self._binary_path}")
            with open(self._binary_path, "rb") as f:
                sys_obj.rust_system.load_binary(f.read(), 0x80000000)
        else:
            if not self._kernel_path:
                print("Warning: No binary or kernel specified.")

        if self._is_kernel_mode:
            print("Starting simulation (progress every 5M cycles; UART = stderr)...")
            sys.stdout.flush()
            try:
                exit_code = run_with_progress(rust_cpu, progress_interval_cycles=5_000_000)
                print(file=sys.stderr)
                stats = rust_cpu.get_stats()
                stats.print()
                return exit_code
            except Exception as e:
                print(file=sys.stderr)
                print(f"Simulation stopped: {e}")
                rust_cpu.get_stats().print()
                raise
        return simulate(rust_cpu)
