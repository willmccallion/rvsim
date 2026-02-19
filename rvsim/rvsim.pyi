"""Type stubs for rvsim."""

from typing import Any, Dict, List, Optional, Sequence, Union

# ── types.py ─────────────────────────────────────────────────────────────────

def _parse_size(s: str | int) -> int: ...

class BranchPredictor:
    class Static:
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

    class GShare:
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

    class TAGE:
        num_banks: int
        table_size: int
        loop_table_size: int
        reset_interval: int
        history_lengths: List[int]
        tag_widths: List[int]
        def __init__(
            self,
            num_banks: int = 4,
            table_size: int = 2048,
            loop_table_size: int = 256,
            reset_interval: int = 2000,
            history_lengths: Optional[List[int]] = None,
            tag_widths: Optional[List[int]] = None,
        ) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

    class Perceptron:
        history_length: int
        table_bits: int
        def __init__(self, history_length: int = 32, table_bits: int = 10) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

    class Tournament:
        global_size_bits: int
        local_hist_bits: int
        local_pred_bits: int
        def __init__(
            self,
            global_size_bits: int = 12,
            local_hist_bits: int = 10,
            local_pred_bits: int = 10,
        ) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

class ReplacementPolicy:
    class LRU:
        def _to_dict_value(self) -> str: ...

    class PLRU:
        def _to_dict_value(self) -> str: ...

    class FIFO:
        def _to_dict_value(self) -> str: ...

    class Random:
        def _to_dict_value(self) -> str: ...

    class MRU:
        def _to_dict_value(self) -> str: ...

class Prefetcher:
    class None_:
        def _to_dict_value(self) -> str: ...
        def _degree(self) -> int: ...
        def _table_size(self) -> int: ...

    class NextLine:
        degree: int
        def __init__(self, degree: int = 1) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _degree(self) -> int: ...
        def _table_size(self) -> int: ...

    class Stride:
        degree: int
        table_size: int
        def __init__(self, degree: int = 1, table_size: int = 64) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _degree(self) -> int: ...
        def _table_size(self) -> int: ...

    class Stream:
        def _to_dict_value(self) -> str: ...
        def _degree(self) -> int: ...
        def _table_size(self) -> int: ...

    class Tagged:
        def _to_dict_value(self) -> str: ...
        def _degree(self) -> int: ...
        def _table_size(self) -> int: ...

class MemoryController:
    class Simple:
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

    class DRAM:
        t_cas: int
        t_ras: int
        t_pre: int
        row_miss_latency: int
        def __init__(
            self,
            t_cas: int = 14,
            t_ras: int = 14,
            t_pre: int = 14,
            row_miss_latency: int = 120,
        ) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _sub_dict(self) -> dict: ...

class Backend:
    class InOrder:
        def _to_dict_value(self) -> str: ...
        def _rob_size(self) -> int: ...
        def _store_buffer_size(self) -> int: ...

    class OutOfOrder:
        rob_size: int
        store_buffer_size: int
        def __init__(self, rob_size: int = 64, store_buffer_size: int = 16) -> None: ...
        def _to_dict_value(self) -> str: ...
        def _rob_size(self) -> int: ...
        def _store_buffer_size(self) -> int: ...

class Cache:
    size_bytes: int
    line_bytes: int
    ways: int
    policy: Any
    latency: int
    prefetcher: Any
    def __init__(
        self,
        size: str | int = "4KB",
        line: str | int = "64B",
        ways: int = 1,
        policy: Any = None,
        latency: int = 1,
        prefetcher: Any = None,
    ) -> None: ...
    def _to_cache_dict(self) -> Dict[str, Any]: ...

# ── config.py ────────────────────────────────────────────────────────────────

class Config:
    width: int
    branch_predictor: Any
    backend: Any
    btb_size: int
    ras_size: int
    l1i: Optional[Cache]
    l1d: Optional[Cache]
    l2: Optional[Cache]
    l3: Optional[Cache]
    ram_size: int
    memory_controller: Any
    tlb_size: int
    trace: bool
    start_pc: int
    direct_mode: bool
    initial_sp: Optional[int]
    ram_base: int
    uart_base: int
    disk_base: int
    clint_base: int
    syscon_base: int
    kernel_offset: int
    bus_width: int
    bus_latency: int
    clint_divider: int
    uart_to_stderr: bool
    def __init__(
        self,
        width: int = 1,
        branch_predictor: Any = None,
        backend: Any = None,
        btb_size: int = 256,
        ras_size: int = 8,
        l1i: Optional[Cache] = None,
        l1d: Optional[Cache] = None,
        l2: Optional[Cache] = None,
        l3: Optional[Cache] = None,
        ram_size: str | int = "256MB",
        memory_controller: Any = None,
        tlb_size: int = 32,
        trace: bool = False,
        start_pc: int = 0x8000_0000,
        direct_mode: bool = True,
        initial_sp: Optional[int] = None,
        ram_base: int = 0x8000_0000,
        uart_base: int = 0x1000_0000,
        disk_base: int = 0x9000_0000,
        clint_base: int = 0x0200_0000,
        syscon_base: int = 0x0010_0000,
        kernel_offset: int = 0x0020_0000,
        bus_width: int = 8,
        bus_latency: int = 4,
        clint_divider: int = 10,
        uart_to_stderr: bool = False,
    ) -> None: ...
    def to_dict(self) -> Dict[str, Any]: ...

# ── stats.py ─────────────────────────────────────────────────────────────────

class Stats(dict):
    def __init__(self, data: Dict[str, Any]) -> None: ...
    def query(self, pattern: str) -> Stats: ...
    def compare(self, other: Stats) -> None: ...

def compare(
    results: Dict[str, Any],
    *,
    metrics: Optional[List[str]] = None,
    baseline: Optional[str] = None,
) -> None: ...

# ── objects.py ───────────────────────────────────────────────────────────────

class _RegisterFile:
    def __getitem__(self, idx: int) -> int: ...
    def __setitem__(self, idx: int, value: int) -> None: ...

class _MemView:
    def __getitem__(self, addr: int) -> int: ...

class Instruction:
    pc: int
    raw: int
    asm: str
    cycles: int
    def __init__(self, pc: int, raw: int, asm: str, cycles: int) -> None: ...

class Cpu:
    def __init__(self, rust_cpu: Any) -> None: ...
    @property
    def pc(self) -> int: ...
    @pc.setter
    def pc(self, value: int) -> None: ...
    @property
    def stats(self) -> Stats: ...
    @property
    def regs(self) -> _RegisterFile: ...
    @property
    def mem32(self) -> _MemView: ...
    @property
    def mem64(self) -> _MemView: ...
    def step(self) -> Optional[Instruction]: ...
    def run(self, limit: Optional[int] = None) -> Optional[int]: ...
    def tick(self) -> None: ...
    def csr(self, name: str | int) -> int: ...
    def get_pc_trace(self) -> list: ...
    def load_kernel(self, kernel_path: str, dtb_path: Optional[str] = None) -> None: ...
    @property
    def raw(self) -> Any: ...

class System:
    rust_system: Any
    def __init__(
        self,
        ram_size: str | int = "128MB",
        ram_base: int = 0x80000000,
        trace: bool = False,
    ) -> None: ...
    def instantiate(
        self,
        disk_image: Optional[str] = None,
        config: Optional[Config | Dict[str, Any]] = None,
    ) -> Any: ...

class Simulator:
    def __init__(self) -> None: ...
    def with_config(self, config: Config) -> Simulator: ...
    def config(self, path: str) -> Simulator: ...
    def kernel(self, path: str) -> Simulator: ...
    def disk(self, path: str) -> Simulator: ...
    def dtb(self, path: str) -> Simulator: ...
    def kernel_mode(self) -> Simulator: ...
    def binary(self, path: str) -> Simulator: ...
    def run(self) -> int: ...

def simulate(
    cpu: Any, print_stats: bool = True, stats_sections: Optional[list] = None
) -> int: ...

# ── experiment.py ────────────────────────────────────────────────────────────

class Environment:
    binary: str
    config: Optional[Union[Config, Dict[str, Any]]]
    disk: Optional[str]
    load_addr: int
    def __init__(
        self,
        binary: str,
        config: Optional[Union[Config, Dict[str, Any]]] = None,
        disk: Optional[str] = None,
        load_addr: int = 0x8000_0000,
    ) -> None: ...
    def get_config(self) -> Dict[str, Any]: ...

class Result:
    exit_code: int
    stats: Stats
    wall_time_sec: float
    binary: str
    def __init__(
        self,
        exit_code: int,
        stats: Stats = ...,
        wall_time_sec: float = 0.0,
        binary: str = "",
    ) -> None: ...
    def to_dict(self) -> Dict[str, Any]: ...

def run_experiment(env: Environment, quiet: bool = True) -> Result: ...

# ── isa.py ───────────────────────────────────────────────────────────────────

class _RegLookup:
    ZERO: int
    RA: int
    SP: int
    GP: int
    TP: int
    T0: int
    T1: int
    T2: int
    S0: int
    FP: int
    S1: int
    A0: int
    A1: int
    A2: int
    A3: int
    A4: int
    A5: int
    A6: int
    A7: int
    S2: int
    S3: int
    S4: int
    S5: int
    S6: int
    S7: int
    S8: int
    S9: int
    S10: int
    S11: int
    T3: int
    T4: int
    T5: int
    T6: int
    def __call__(self, name: str | int) -> int: ...

class _CsrLookup:
    SSTATUS: int
    SIE: int
    STVEC: int
    SSCRATCH: int
    SEPC: int
    SCAUSE: int
    STVAL: int
    SIP: int
    SATP: int
    MSTATUS: int
    MISA: int
    MEDELEG: int
    MIDELEG: int
    MIE: int
    MTVEC: int
    MSCRATCH: int
    MEPC: int
    MCAUSE: int
    MTVAL: int
    MIP: int
    CYCLE: int
    TIME: int
    INSTRET: int
    MCYCLE: int
    MINSTRET: int
    STIMECMP: int
    def __call__(self, name: str | int) -> int: ...

reg: _RegLookup
csr: _CsrLookup

def reg_name(idx: int) -> str: ...
def csr_name(addr: int) -> str: ...
def disassemble(inst: int) -> str: ...
def version() -> str: ...
