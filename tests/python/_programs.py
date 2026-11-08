"""Guest programs the Python tests run: a static riscv64 ELF built around
a few hand-encoded instructions, and the programs several tests share.

Standalone (no rvsim import), so a wheel can be checked against them.
"""

import struct

LOAD_ADDRESS = 0x1_0000
CODE_OFFSET = 0x80
ECALL = 0x0000_0073
SYS_EXIT = 93
SYS_WRITE = 64
M5_FAIL = 0x22


def addi(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (rd << 7) | 0x13


def ld(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (3 << 12) | (rd << 7) | 0x03


def auipc(rd: int, imm: int) -> int:
    return ((imm & 0xFFFFF) << 12) | (rd << 7) | 0x17


def branch(funct3: int, rs1: int, rs2: int, imm: int) -> int:
    imm &= 0x1FFF
    return (
        (((imm >> 12) & 1) << 31)
        | (((imm >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (funct3 << 12)
        | (((imm >> 1) & 0xF) << 8)
        | (((imm >> 11) & 1) << 7)
        | 0x63
    )


def blt(rs1: int, rs2: int, imm: int) -> int:
    return branch(4, rs1, rs2, imm)


def beq(rs1: int, rs2: int, imm: int) -> int:
    return branch(0, rs1, rs2, imm)


def lui(rd: int, imm20: int) -> int:
    return ((imm20 & 0xFFFFF) << 12) | (rd << 7) | 0x37


def sub(rd: int, rs1: int, rs2: int) -> int:
    return (0x20 << 25) | (rs2 << 20) | (rs1 << 15) | (rd << 7) | 0x33


def lw(rd: int, rs1: int, imm: int) -> int:
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (2 << 12) | (rd << 7) | 0x03


def sw(rs2: int, rs1: int, imm: int) -> int:
    imm &= 0xFFF
    return (
        ((imm >> 5) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (2 << 12)
        | ((imm & 0x1F) << 7)
        | 0x23
    )


def user_elf(code: list[int]) -> bytes:
    """A static riscv64 executable with one RWX segment holding ``code``."""
    entry = LOAD_ADDRESS + CODE_OFFSET
    header = b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8)
    header += struct.pack(
        "<HHIQQQIHHHHHH", 2, 243, 1, entry, 64, 0, 0, 64, 56, 1, 0, 0, 0
    )
    size = CODE_OFFSET + 4 * len(code)
    phdr = struct.pack(
        "<IIQQQQQQ", 1, 7, 0, LOAD_ADDRESS, LOAD_ADDRESS, size, size, 0x1000
    )
    image = header + phdr
    image += bytes(CODE_OFFSET - len(image))
    image += b"".join(struct.pack("<I", word) for word in code)
    return image


# argc from the stack into a0, then exit(a0).
EXIT_WITH_ARGC = [ld(10, 2, 0), addi(17, 0, SYS_EXIT), ECALL]
# Counts to 300 in t0, then exit(9).
COUNT_THEN_EXIT = [
    addi(5, 0, 0),
    addi(6, 0, 300),
    addi(5, 5, 1),
    blt(5, 6, -4),
    addi(10, 0, 9),
    addi(17, 0, SYS_EXIT),
    ECALL,
]


M5_READ_FILE = 0x50
M5_WRITE_FILE = 0x4F
BUFFER_OFFSET = 0x100
NAME_OFFSET = 0x180
SCRIPT = b"echo from the host\n"


def file_exchange() -> list[int]:
    """Reads the script into a buffer past the code, writes the buffer to
    ``out.txt`` in the output directory, and exits with the bytes written."""
    code = [
        auipc(10, 0),
        addi(10, 10, BUFFER_OFFSET),
        addi(11, 0, 64),
        addi(12, 0, 0),
        0x7B | (M5_READ_FILE << 25),
        addi(5, 10, 0),
        auipc(10, 0),
        addi(10, 10, BUFFER_OFFSET - 6 * 4),
        addi(11, 5, 0),
        addi(12, 0, 0),
        auipc(13, 0),
        addi(13, 13, NAME_OFFSET - 10 * 4),
        0x7B | (M5_WRITE_FILE << 25),
        addi(11, 10, 0),
        0x7B | (M5_FAIL << 25),
    ]
    code += [0] * (NAME_OFFSET // 4 - len(code))
    name = b"out.txt\0"
    code += [int.from_bytes(name[i : i + 4], "little") for i in range(0, 8, 4)]
    return code
