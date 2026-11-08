"""
RISC-V ABI register and CSR definitions.

Provides callable namespace objects:
- ``reg``: Register lookup (``reg.RA`` → 1, ``reg("ra")`` → 1, ``reg.name(5)`` → ``"t0"``)
- ``csr``: CSR lookup (``csr.MSTATUS`` → 0x300, ``csr("mstatus")`` → 0x300, ``csr.name(0x300)`` → ``"mstatus"``)
"""

import struct
import sys
from enum import Enum

from ._core import csr_names, disassemble, register_names

__all__ = ["Disassemble", "Privilege", "csr", "reg"]


class Privilege(str, Enum):
    """A RISC-V privilege mode, by the letter the spec gives it; equal to
    that letter as a string."""

    MACHINE = "M"
    SUPERVISOR = "S"
    USER = "U"

    def __str__(self) -> str:
        return self.value


class Disassemble:
    """Fluent disassembler for RISC-V binaries and raw bytes.

    Usage::

        Disassemble().binary("software/bin/programs/qsort.bin").print()
        Disassemble().binary("qsort.bin").at(0x80000024, count=10).print()
        Disassemble().bytes(data).print()
        Disassemble().inst(0x00a00513)  # single instruction -> str
    """

    def __init__(self):
        self._data: bytes | None = None
        self._base: int = 0x8000_0000
        self._offset: int = 0
        self._count: int | None = None

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

    def at(self, addr: int, count: int | None = None) -> "Disassemble":
        self._offset = addr - self._base
        self._count = count
        return self

    def limit(self, n: int) -> "Disassemble":
        self._count = n
        return self

    def inst(self, raw: int) -> str:
        return disassemble(raw)

    def decode(self) -> list[tuple[int, int, str]]:
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


_REG_NAMES: list[str] = register_names()

_REG_BY_NAME: dict[str, int] = {name: idx for idx, name in enumerate(_REG_NAMES)}
_REG_BY_NAME["fp"] = 8
for _i in range(32):
    _REG_BY_NAME[f"x{_i}"] = _i


class _RegLookup:
    """Callable register lookup, with each register as an upper-case
    attribute.

    Usage::

        reg.RA        # 1
        reg.SP        # 2
        reg("ra")     # 1
        reg("x5")    # 5
    """

    def __getattr__(self, name: str) -> int:
        try:
            return _REG_BY_NAME[name.lower()]
        except KeyError:
            raise AttributeError(f"no register named {name!r}") from None

    def __call__(self, name) -> int:
        if isinstance(name, int):
            return name
        return _REG_BY_NAME[name.lower()]

    def name(self, idx: int) -> str:
        """Return the ABI name for a register index (e.g. ``reg.name(5)`` → ``"t0"``)."""
        return _REG_NAMES[idx]

    def __repr__(self) -> str:
        return "reg"


reg = _RegLookup()


_CSR_BY_NAME: dict[str, int] = dict(csr_names())
_CSR_BY_ADDR: dict[int, str] = {addr: name for name, addr in _CSR_BY_NAME.items()}


class _CsrLookup:
    """Callable CSR lookup, with each CSR the simulator implements as an
    upper-case attribute.

    Usage::

        csr.MSTATUS    # 0x300
        csr.SATP       # 0x180
        csr("mstatus") # 0x300
    """

    def __getattr__(self, name: str) -> int:
        try:
            return _CSR_BY_NAME[name.lower()]
        except KeyError:
            raise AttributeError(f"no CSR named {name!r}") from None

    def __call__(self, name) -> int:
        if isinstance(name, int):
            return name
        return _CSR_BY_NAME[name.lower()]

    def name(self, addr: int) -> str:
        """Return the CSR name for an address (e.g. ``csr.name(0x300)`` → ``"mstatus"``)."""
        return _CSR_BY_ADDR.get(addr, f"csr_{addr:#05x}")

    def __repr__(self) -> str:
        return "csr"


csr = _CsrLookup()
