"""The comparison's results: each simulator's stats for every variant and
program, kept as `results/<simulator>.json` beside this file."""

import json
import sys
from pathlib import Path

RESULTS_DIR = Path(__file__).parent / "results"
PROGRAMS = Path(__file__).resolve().parents[2] / "tests/builds/compare-programs"

# One program's stats under one variant, by the names compare.py reads; a
# stat the simulator could not report is None, and a failed run has none.
ProgramStats = dict[str, float | None]
# Variant name -> program name -> stats.
Results = dict[str, dict[str, ProgramStats]]


def program_binaries() -> list[Path]:
    """Every comparison program, or exit if none has been built."""
    binaries = sorted(PROGRAMS.glob("*.elf"))
    if not binaries:
        sys.exit(
            f"error: no programs in {PROGRAMS}; run tools/gem5_compare/programs/build.sh"
        )
    return binaries


def load(simulator: str) -> Results:
    """The results saved for `simulator`, or none if it has not run."""
    path = RESULTS_DIR / f"{simulator}.json"
    if not path.exists():
        return {}
    saved: Results = json.loads(path.read_text())
    return saved


def save(simulator: str, results: Results) -> Path:
    """Saves `results` for `simulator`; variants not in them keep the results
    saved before. Returns the file written."""
    merged = load(simulator)
    merged.update(results)
    RESULTS_DIR.mkdir(exist_ok=True)
    path = RESULTS_DIR / f"{simulator}.json"
    path.write_text(json.dumps(merged, indent=2, sort_keys=True))
    return path
