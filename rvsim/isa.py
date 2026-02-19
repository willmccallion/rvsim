"""
RISC-V ABI register and CSR definitions.

Provides callable namespace objects:
- ``reg``: Register lookup (``reg.RA`` → 1, ``reg("ra")`` → 1, ``reg("x5")`` → 5)
- ``csr``: CSR lookup (``csr.MSTATUS`` → 0x300, ``csr("mstatus")`` → 0x300)
- ``reg_name(idx)``: Index to ABI name (``reg_name(5)`` → ``"t0"``)
- ``csr_name(addr)``: Address to name (``csr_name(0x300)`` → ``"mstatus"``)
"""

import struct
import sys
from typing import List, Optional, Tuple

from ._core import disassemble as disassemble  # noqa: F401


class Disassemble:
    """Fluent disassembler for RISC-V binaries and raw bytes.

    Usage::

        Disassemble().binary("software/bin/programs/qsort.bin").print()
        Disassemble().binary("qsort.bin").at(0x80000024, count=10).print()
        Disassemble().bytes(data).print()
        Disassemble().inst(0x00a00513)  # single instruction -> str
    """

    def __init__(self):
        self._data: Optional[bytes] = None
        self._base: int = 0x8000_0000
        self._offset: int = 0
        self._count: Optional[int] = None

    def binary(self, path: str) -> "Disassemble":
        with open(path, "rb") as f:
            self._data = f.read()
        return self

    def bytes(self, data: bytes) -> "Disassemble":
        self._data = data
        return self

    def base(self, addr: int) -> "Disassemble":
        self._base = addr
        return self

    def at(self, addr: int, count: Optional[int] = None) -> "Disassemble":
        self._offset = addr - self._base
        self._count = count
        return self

    def limit(self, n: int) -> "Disassemble":
        self._count = n
        return self

    def inst(self, raw: int) -> str:
        return disassemble(raw)

    def decode(self) -> List[Tuple[int, int, str]]:
        if self._data is None:
            raise ValueError(
                "No data to disassemble. Call .binary() or .bytes() first."
            )
        start = max(0, self._offset)
        start = start & ~1  # align to 2 (smallest RVC instruction)
        end = len(self._data)
        result = []
        off = start
        while off < end - 1:
            half = struct.unpack_from("<H", self._data, off)[0]
            if half & 0x3 != 0x3:
                # 16-bit compressed instruction
                asm = disassemble(half)
                result.append((self._base + off, half, asm))
                off += 2
            elif off + 3 < end:
                # 32-bit instruction
                inst = struct.unpack_from("<I", self._data, off)[0]
                asm = disassemble(inst)
                result.append((self._base + off, inst, asm))
                off += 4
            else:
                break
            if self._count is not None and len(result) >= self._count:
                break
        return result

    def print(self, file=None) -> "Disassemble":
        if file is None:
            file = sys.stdout
        for pc, raw, asm in self.decode():
            width = 4 if raw > 0xFFFF else 8
            print(f"0x{pc:08x}  {raw:0{width}x}  {asm}", file=file)
        return self


# ── Register name ↔ index helpers ────────────────────────────────────────────

_REG_NAMES: list[str] = [
    "zero",
    "ra",
    "sp",
    "gp",
    "tp",
    "t0",
    "t1",
    "t2",
    "s0",
    "s1",
    "a0",
    "a1",
    "a2",
    "a3",
    "a4",
    "a5",
    "a6",
    "a7",
    "s2",
    "s3",
    "s4",
    "s5",
    "s6",
    "s7",
    "s8",
    "s9",
    "s10",
    "s11",
    "t3",
    "t4",
    "t5",
    "t6",
]

_REG_BY_NAME: dict[str, int] = {name: idx for idx, name in enumerate(_REG_NAMES)}
_REG_BY_NAME["fp"] = 8
for _i in range(32):
    _REG_BY_NAME[f"x{_i}"] = _i


class _RegLookup:
    """Callable register lookup with attribute constants.

    Usage::

        reg.RA        # 1
        reg.SP        # 2
        reg("ra")     # 1
        reg("x5")    # 5
    """

    ZERO = 0
    RA = 1
    SP = 2
    GP = 3
    TP = 4
    T0 = 5
    T1 = 6
    T2 = 7
    S0 = 8
    FP = 8
    S1 = 9
    A0 = 10
    A1 = 11
    A2 = 12
    A3 = 13
    A4 = 14
    A5 = 15
    A6 = 16
    A7 = 17
    S2 = 18
    S3 = 19
    S4 = 20
    S5 = 21
    S6 = 22
    S7 = 23
    S8 = 24
    S9 = 25
    S10 = 26
    S11 = 27
    T3 = 28
    T4 = 29
    T5 = 30
    T6 = 31

    def __call__(self, name) -> int:
        if isinstance(name, int):
            return name
        return _REG_BY_NAME[name.lower()]

    def __repr__(self) -> str:
        return "reg"


reg = _RegLookup()


def reg_name(idx: int) -> str:
    """Return the ABI name for a register index (e.g. ``reg_name(5)`` → ``"t0"``)."""
    return _REG_NAMES[idx]


# ── CSR name ↔ address helpers ───────────────────────────────────────────────

_CSR_BY_NAME: dict[str, int] = {
    # Supervisor
    "sstatus": 0x100,
    "sie": 0x104,
    "stvec": 0x105,
    "sscratch": 0x140,
    "sepc": 0x141,
    "scause": 0x142,
    "stval": 0x143,
    "sip": 0x144,
    "satp": 0x180,
    # Machine
    "mstatus": 0x300,
    "misa": 0x301,
    "medeleg": 0x302,
    "mideleg": 0x303,
    "mie": 0x304,
    "mtvec": 0x305,
    "mscratch": 0x340,
    "mepc": 0x341,
    "mcause": 0x342,
    "mtval": 0x343,
    "mip": 0x344,
    # Counters
    "cycle": 0xC00,
    "time": 0xC01,
    "instret": 0xC02,
    "mcycle": 0xB00,
    "minstret": 0xB02,
    # Stimecmp (Sstc extension)
    "stimecmp": 0x14D,
}

_CSR_BY_ADDR: dict[int, str] = {addr: name for name, addr in _CSR_BY_NAME.items()}


class _CsrLookup:
    """Callable CSR lookup with attribute constants.

    Usage::

        csr.MSTATUS    # 0x300
        csr.SATP       # 0x180
        csr("mstatus") # 0x300
    """

    # Supervisor
    SSTATUS = 0x100
    SIE = 0x104
    STVEC = 0x105
    SSCRATCH = 0x140
    SEPC = 0x141
    SCAUSE = 0x142
    STVAL = 0x143
    SIP = 0x144
    SATP = 0x180
    # Machine
    MSTATUS = 0x300
    MISA = 0x301
    MEDELEG = 0x302
    MIDELEG = 0x303
    MIE = 0x304
    MTVEC = 0x305
    MSCRATCH = 0x340
    MEPC = 0x341
    MCAUSE = 0x342
    MTVAL = 0x343
    MIP = 0x344
    # Counters
    CYCLE = 0xC00
    TIME = 0xC01
    INSTRET = 0xC02
    MCYCLE = 0xB00
    MINSTRET = 0xB02
    # Sstc
    STIMECMP = 0x14D

    def __call__(self, name) -> int:
        if isinstance(name, int):
            return name
        return _CSR_BY_NAME[name.lower()]

    def __repr__(self) -> str:
        return "csr"


csr = _CsrLookup()


def csr_name(addr: int) -> str:
    """Return the CSR name for an address (e.g. ``csr_name(0x300)`` → ``"mstatus"``)."""
    return _CSR_BY_ADDR.get(addr, f"csr_{addr:#05x}")
