"""Checks SE mode against qemu-riscv64: the same static Linux program,
run under both, must print the same output and exit with the same code.

    .venv/bin/python tools/se/verify.py [--embench] [--bundled [--slow]] [--programs GLOB ... [--harts N]]

The check programs (tools/se/programs, built by `make se-programs`) run
by default; `--embench` adds the Embench-IoT benchmarks (`make
embench-build`) and `--bundled` the bundled programs and benchmarks
(`make software`), whose cycle counts are left out of the comparison,
whose minutes-long members only run with `--slow`, and whose `regions`
qemu cannot run, since it uses the pseudo-instruction.
Each rvsim run is a subprocess so its stdout is its program's alone.
Exits non-zero when any program differs.
"""

from __future__ import annotations

import argparse
import glob
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SE_PROGRAMS = ROOT / "tests" / "builds" / "se-programs"
EMBENCH = ROOT / "tests" / "builds" / "embench" / "bin"
BUNDLED = ROOT / "software" / "bin"
# Bundled programs that take minutes under rvsim: sand about fifteen, life
# about five. --bundled leaves them to --slow.
LONG_RUNNING = {"life", "sand"}
# Bundled programs qemu cannot run: regions uses the pseudo-instruction,
# which qemu traps on as illegal.
NOT_UNDER_QEMU = {"regions"}
PYTHON = ROOT / ".venv" / "bin" / "python"
CYCLE_LIMIT = 2_000_000_000

RUNNER = """
import os
import sys
from rvsim import Config, Simulator
harts, binary, args = int(sys.argv[1]), sys.argv[2], sys.argv[3:]
env = [f"{{name}}={{value}}" for name, value in os.environ.items()]
cpu = Simulator(Config(hart_count=harts), binary=binary, args=[binary, *args], env=env)
code = cpu.run(limit={limit})
sys.stdout.flush()
sys.exit(255 if code is None else int(code))
"""


@dataclass
class Program:
    """One program to run under both, with its arguments, its environment
    and, under rvsim, a hart for each thread it runs at once."""

    elf: Path
    args: list[str] = field(default_factory=list)
    env: dict[str, str] = field(default_factory=dict)
    harts: int = 1

    @property
    def name(self) -> str:
        return self.elf.stem


@dataclass
class Outcome:
    stdout: bytes
    exit_code: int

    @property
    def result(self) -> bytes:
        """The output without the cycle counts, which are the measurement
        rather than the program's result and differ by design."""
        return b"\n".join(
            line for line in self.stdout.split(b"\n") if b"Cycles:" not in line
        )


def run_qemu(program: Program, workdir: Path) -> Outcome:
    result = subprocess.run(
        ["qemu-riscv64", str(program.elf), *program.args],
        cwd=workdir,
        stdin=subprocess.DEVNULL,
        env={**os.environ, **program.env},
        capture_output=True,
        check=False,
        timeout=600,
    )
    return Outcome(result.stdout, result.returncode)


def run_rvsim(program: Program, workdir: Path) -> Outcome:
    result = subprocess.run(
        [
            str(PYTHON),
            "-c",
            RUNNER.format(limit=CYCLE_LIMIT),
            str(program.harts),
            str(program.elf),
            *program.args,
        ],
        cwd=workdir,
        stdin=subprocess.DEVNULL,
        env={**os.environ, **program.env},
        capture_output=True,
        check=False,
        timeout=3600,
    )
    if result.returncode == 255 and b"Traceback" in result.stderr:
        sys.stderr.write(result.stderr.decode(errors="replace"))
    return Outcome(result.stdout, result.returncode)


def check(program: Program) -> tuple[Program, Outcome, Outcome]:
    with (
        tempfile.TemporaryDirectory() as qemu_dir,
        tempfile.TemporaryDirectory() as rvsim_dir,
    ):
        expected = run_qemu(program, Path(qemu_dir))
        actual = run_rvsim(program, Path(rvsim_dir))
    return program, expected, actual


# The arguments, environment and harts programs expect, by name: threads
# runs four workers beside its first thread; raytracer_mt renders on four
# threads, given the count so that qemu, which reports the host's CPUs,
# starts as many.
SPECS = {
    "hello": Program(Path(), ["7", "second argument"], {"SE_GREETING": "hi"}),
    "hello_pie": Program(Path(), ["7", "second argument"], {"SE_GREETING": "hi"}),
    "files": Program(Path(), ["."]),
    "threads": Program(Path(), harts=5),
    "raytracer_mt": Program(Path(), ["4"], harts=4),
}


def program_for(elf: Path) -> Program:
    """`elf` with the arguments, environment and harts it expects."""
    spec = SPECS.get(elf.stem, Program(Path()))
    return Program(elf, spec.args, spec.env, spec.harts)


def check_programs() -> list[Program]:
    """The check programs, set up as they expect."""
    return [program_for(elf) for elf in sorted(SE_PROGRAMS.glob("*.elf"))]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--embench", action="store_true", help="also run the Embench-IoT benchmarks"
    )
    parser.add_argument(
        "--bundled", action="store_true", help="also run the bundled programs"
    )
    parser.add_argument(
        "--slow",
        action="store_true",
        help="include the bundled programs that take minutes (life, sand)",
    )
    parser.add_argument(
        "--programs",
        nargs="*",
        default=[],
        help="ELF globs to run instead of the defaults",
    )
    parser.add_argument(
        "--harts",
        type=int,
        default=1,
        help="harts to give the --programs runs, one per thread they run at once",
    )
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()

    programs: list[Program] = []
    if args.programs:
        for pattern in args.programs:
            programs.extend(
                Program(Path(p).resolve(), harts=args.harts)
                for p in sorted(glob.glob(pattern))
            )
    else:
        programs = check_programs()
        if args.embench:
            programs.extend(Program(elf) for elf in sorted(EMBENCH.glob("*.elf")))
        if args.bundled:
            for subdir in ("programs", "benchmarks"):
                programs.extend(
                    program_for(elf)
                    for elf in sorted((BUNDLED / subdir).glob("*.elf"))
                    if elf.stem not in NOT_UNDER_QEMU
                    and (args.slow or elf.stem not in LONG_RUNNING)
                )
    if not programs:
        print(
            "no programs to run; build them with make se-programs, make embench-build or make software"
        )
        return 2

    failures = 0
    print(f"{'program':<20} {'qemu':>5} {'rvsim':>6}  result")
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for program, expected, actual in pool.map(check, programs):
            same = (
                expected.result == actual.result
                and expected.exit_code == actual.exit_code
            )
            verdict = "same" if same else "DIFFERS"
            if not same:
                failures += 1
                if expected.result != actual.result:
                    verdict += " (stdout)"
            print(
                f"{program.name:<20} {expected.exit_code:>5} {actual.exit_code:>6}  {verdict}"
            )
            if not same and expected.result != actual.result:
                print(
                    "  qemu : "
                    + expected.stdout[:300]
                    .decode(errors="replace")
                    .replace("\n", "\\n")
                )
                print(
                    "  rvsim: "
                    + actual.stdout[:300].decode(errors="replace").replace("\n", "\\n")
                )
    print(f"\n{len(programs) - failures}/{len(programs)} programs match qemu-riscv64")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
