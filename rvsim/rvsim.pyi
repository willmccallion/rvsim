"""Type stubs for rvsim."""

from typing import Any, Dict, List, Optional, Union

# ── pipeline.py ───────────────────────────────────────────────────────────────

class PipelineSnapshot:
    """Point-in-time snapshot of all pipeline inter-stage latches.

    Obtained via ``cpu.pipeline_snapshot()`` after any ``tick()`` or ``step()``.
    Each stage attribute is a list of slot dicts (length ≤ ``width``).
    """

    width: int
    fetch1_fetch2: List[Dict[str, Any]]
    fetch2_decode: List[Dict[str, Any]]
    decode_rename: List[Dict[str, Any]]
    rename_issue: List[Dict[str, Any]]
    issue_queue: List[Dict[str, Any]]
    execute_mem1: List[Dict[str, Any]]
    mem1_mem2: List[Dict[str, Any]]
    mem2_wb: List[Dict[str, Any]]
    fetch1_stall: int
    fetch2_stall: int
    mem1_stall: int

    def render(self) -> str:
        """Return the pipeline diagram as a string."""
        ...

    def visualize(self) -> None:
        """Print the pipeline diagram to stdout."""
        ...

# ── types.py ─────────────────────────────────────────────────────────────────

class BranchPredictor:
    class Static: ...

    class GShare: ...

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

    class Perceptron:
        history_length: int
        table_bits: int
        def __init__(self, history_length: int = 32, table_bits: int = 10) -> None: ...

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

class ReplacementPolicy:
    class LRU: ...
    class PLRU: ...
    class FIFO: ...
    class Random: ...
    class MRU: ...

class Prefetcher:
    class Off: ...

    class NextLine:
        degree: int
        def __init__(self, degree: int = 1) -> None: ...

    class Stride:
        degree: int
        table_size: int
        def __init__(self, degree: int = 1, table_size: int = 64) -> None: ...

    class Stream: ...
    class Tagged: ...

class MemoryController:
    class Simple: ...

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

class Fu:
    class IntAlu:
        count: int
        latency: int
        def __init__(self, count: int = 4, latency: int = 1) -> None: ...
    class IntMul:
        count: int
        latency: int
        def __init__(self, count: int = 1, latency: int = 3) -> None: ...
    class IntDiv:
        count: int
        latency: int
        def __init__(self, count: int = 1, latency: int = 35) -> None: ...
    class FpAdd:
        count: int
        latency: int
        def __init__(self, count: int = 2, latency: int = 4) -> None: ...
    class FpMul:
        count: int
        latency: int
        def __init__(self, count: int = 2, latency: int = 5) -> None: ...
    class FpFma:
        count: int
        latency: int
        def __init__(self, count: int = 2, latency: int = 5) -> None: ...
    class FpDivSqrt:
        count: int
        latency: int
        def __init__(self, count: int = 1, latency: int = 21) -> None: ...
    class Branch:
        count: int
        latency: int
        def __init__(self, count: int = 2, latency: int = 1) -> None: ...
    class Mem:
        count: int
        latency: int
        def __init__(self, count: int = 2, latency: int = 1) -> None: ...

    units: List[Any]
    def __init__(self, units: Optional[List[Any]] = None) -> None: ...

class Backend:
    class InOrder: ...

    class OutOfOrder:
        rob_size: int
        store_buffer_size: int
        issue_queue_size: int
        load_queue_size: int
        load_ports: int
        store_ports: int
        prf_gpr_size: int
        prf_fpr_size: int
        fu_config: Fu
        def __init__(
            self,
            rob_size: int = 128,
            store_buffer_size: int = 32,
            issue_queue_size: int = 32,
            load_queue_size: int = 32,
            load_ports: int = 2,
            store_ports: int = 1,
            prf_gpr_size: int = 256,
            prf_fpr_size: int = 128,
            fu_config: Optional[Fu] = None,
        ) -> None: ...

class Cache:
    size_bytes: int
    line_bytes: int
    ways: int
    policy: Any
    latency: int
    prefetcher: Any
    mshr_count: int
    def __init__(
        self,
        size: str | int = "4KB",
        line: str | int = "64B",
        ways: int = 1,
        policy: Any = None,
        latency: int = 1,
        prefetcher: Any = None,
        mshr_count: int = 0,
    ) -> None: ...

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
    uart_quiet: bool
    def __init__(
        self,
        width: int = 1,
        branch_predictor: Any = None,
        backend: Any = None,
        btb_size: int = 4096,
        ras_size: int = 32,
        l1i: Optional[Cache] = None,
        l1d: Optional[Cache] = None,
        l2: Optional[Cache] = None,
        l3: Optional[Cache] = None,
        ram_size: str | int = "256MB",
        memory_controller: Any = None,
        tlb_size: int = 32,
        trace: bool = False,
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
        uart_quiet: bool = False,
    ) -> None: ...
    def to_dict(self) -> Dict[str, Any]: ...
    def replace(self, **kwargs: Any) -> Config: ...

# ── stats.py ─────────────────────────────────────────────────────────────────

class Stats(dict):
    def __init__(self, data: Dict[str, Any]) -> None: ...
    def query(self, pattern: str) -> Stats: ...
    def compare(self, other: Stats) -> None: ...
    @staticmethod
    def tabulate(rows: Dict[str, Stats], *, title: str = "") -> Table: ...

class Table:
    def __repr__(self) -> str: ...
    def __str__(self) -> str: ...

# ── objects.py ───────────────────────────────────────────────────────────────

class Instruction:
    pc: int
    raw: int
    asm: str
    cycles: int
    def __init__(self, pc: int, raw: int, asm: str, cycles: int) -> None: ...

class Cpu:
    def __init__(
        self,
        config_dict: Dict[str, Any],
        *,
        elf_data: Optional[bytes] = None,
        kernel_path: Optional[str] = None,
        dtb_path: Optional[str] = None,
        disk_path: Optional[str] = None,
    ) -> None: ...
    @property
    def pc(self) -> int: ...
    @pc.setter
    def pc(self, value: int) -> None: ...
    @property
    def privilege(self) -> str: ...
    @property
    def trace(self) -> bool: ...
    @trace.setter
    def trace(self, value: bool) -> None: ...
    @property
    def stats(self) -> Dict[str, Any]: ...
    @property
    def regs(self) -> Registers: ...
    @property
    def csrs(self) -> Csrs: ...
    @property
    def mem32(self) -> Memory: ...
    @property
    def mem64(self) -> Memory: ...
    @property
    def pc_trace(self) -> list[tuple[int, int]]: ...
    def step(self, max_cycles: int = 100_000) -> Optional[Instruction]: ...
    def run(
        self,
        limit: Optional[int] = None,
        progress: int = 0,
        stats_sections: Optional[list[str]] = None,
    ) -> Optional[int]: ...
    def sample(self, every: int, limit: Optional[int] = None) -> list[dict]: ...
    def run_until(
        self,
        predicate: Any = None,
        *,
        pc: Optional[int] = None,
        privilege: Optional[str] = None,
        limit: Optional[int] = None,
        chunk: int = 10_000,
    ) -> Optional[int]: ...
    def tick(self) -> None: ...
    def pipeline_snapshot(self) -> PipelineSnapshot: ...
    def save(self, path: str) -> None: ...
    def restore(self, path: str) -> None: ...

class Registers:
    def __getitem__(self, idx: int) -> int: ...
    def __setitem__(self, idx: int, value: int) -> None: ...

class Csrs:
    def __getitem__(self, key: str | int) -> int: ...

class Memory:
    def __getitem__(self, addr: int) -> int: ...

class Simulator:
    def __init__(self) -> None: ...
    def config(self, path_or_config: Config | str) -> Simulator: ...
    def kernel(self, path: str) -> Simulator: ...
    def disk(self, path: str) -> Simulator: ...
    def dtb(self, path: str) -> Simulator: ...
    def binary(self, path: str) -> Simulator: ...
    def build(self) -> Cpu: ...
    def run(
        self,
        limit: Optional[int] = None,
        progress: int = 0,
        stats_sections: Optional[list[str]] = None,
        output_stats: Optional[str] = None,
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
    def run(
        self, quiet: bool = True, limit: Optional[int] = None, progress: int = 0
    ) -> Result: ...

class Result:
    exit_code: int
    stats: Stats
    wall_time_sec: float
    binary: str
    @property
    def ok(self) -> bool: ...
    def __init__(
        self,
        exit_code: int,
        stats: Stats = ...,
        wall_time_sec: float = 0.0,
        binary: str = "",
    ) -> None: ...
    def to_dict(self) -> Dict[str, Any]: ...
    @staticmethod
    def compare(
        results: Dict[str, Any],
        *,
        metrics: Optional[List[str]] = None,
        baseline: Optional[str] = None,
        col_header: str = "",
    ) -> None: ...

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
    def name(self, idx: int) -> str: ...
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
    def name(self, addr: int) -> str: ...
    def __call__(self, name: str | int) -> int: ...

reg: _RegLookup
csr: _CsrLookup

def version() -> str: ...
