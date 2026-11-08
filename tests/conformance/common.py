"""What the conformance runners share: where the test builds and spike
live, the signature region a test writes, and commit-trace parsing."""

import concurrent.futures as cf
import json
import os
import re
import struct
import subprocess
import sys
from collections import Counter
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BUILDS = os.path.join(ROOT, "tests", "builds")
SPIKE = os.path.join(BUILDS, "spike-install", "bin", "spike")
READELF = "riscv64-elf-readelf"

# One retired instruction in a spike -l / rvsim commit log: its PC and bits.
LINE_RE = re.compile(r"core\s+\d+:\s+0x([0-9a-f]+)\s+\(0x([0-9a-f]+)\)")
# addi x0, x0, 0 and c.nop.
NOP_ENCODINGS = {0x00000013, 0x00000001}
# spike's boot ROM, which rvsim does not run.
SPIKE_RESET_START = 0x1000
SPIKE_RESET_END = 0x2000


def signature_range(elf_path):
    """The (begin, end) addresses of the ELF's begin_signature and
    end_signature symbols, either None when the ELF lacks it."""
    out = subprocess.run(
        [READELF, "-s", elf_path], capture_output=True, text=True, check=True
    ).stdout
    begin = end = None
    for line in out.splitlines():
        parts = line.split()
        if len(parts) >= 8:
            sym = parts[-1]
            if sym == "begin_signature":
                begin = int(parts[1], 16)
            elif sym == "end_signature":
                end = int(parts[1], 16)
    return begin, end


def read_signature(path):
    """A +signature file, one hex word per line, as bytes."""
    out = bytearray()
    with open(path) as f:
        for line in f:
            line = line.strip()
            if line:
                out.extend(struct.pack("<I", int(line, 16)))
    return bytes(out)


def parse_log(path, skip_nops=True, skip_reset=False, max_entries=0):
    """A commit log as (pc, inst) tuples, at most ``max_entries`` of them
    when it is non-zero."""
    entries = []
    with open(path) as f:
        for line in f:
            m = LINE_RE.search(line)
            if m is None:
                continue
            pc = int(m.group(1), 16)
            inst = int(m.group(2), 16)
            if skip_reset and SPIKE_RESET_START <= pc < SPIKE_RESET_END:
                continue
            if skip_nops and inst in NOP_ENCODINGS:
                continue
            entries.append((pc, inst))
            if max_entries and len(entries) >= max_entries:
                break
    return entries


def compare_traces(spike_trace, rvsim_trace):
    """Compares two traces by PC. Returns (match_count, divergence_info_or_None).

    Only PCs are compared, not instruction encodings, because rvsim logs
    the 32-bit expanded form of compressed instructions while spike logs
    the original 16-bit encoding.

    rvsim may be shorter than spike because rvsim exits immediately on
    HTIF tohost write, while spike continues the polling loop.  A shorter
    rvsim trace is fine as long as every PC it *did* retire matches spike
    exactly.  A shorter *spike* trace (rvsim retired instructions spike
    didn't) is a real divergence.
    """
    for i, (s, r) in enumerate(zip(spike_trace, rvsim_trace, strict=False)):
        if s[0] != r[0]:
            return i, {
                "inst_num": i + 1,
                "spike_pc": s[0],
                "spike_inst": s[1],
                "rvsim_pc": r[0],
                "rvsim_inst": r[1],
            }
    min_len = min(len(spike_trace), len(rvsim_trace))
    if len(rvsim_trace) > len(spike_trace):
        return min_len, {
            "inst_num": min_len + 1,
            "length_mismatch": True,
            "spike_len": len(spike_trace),
            "rvsim_len": len(rvsim_trace),
        }
    return min_len, None


def select_pipelines(pipelines, labels=None, smoke=None):
    """The `pipelines` entries named in the comma-separated `labels` (all
    when None), or those in `smoke` when it is given. Exits naming any
    unknown label."""
    if smoke is not None:
        return [(label, cfg) for label, cfg in pipelines if label in smoke]
    if not labels:
        return list(pipelines)
    wanted = {label.strip() for label in labels.split(",")}
    selected = [(label, cfg) for label, cfg in pipelines if label in wanted]
    missing = wanted - {label for label, _ in selected}
    if missing:
        sys.exit(f"unknown pipeline label(s): {sorted(missing)}")
    return selected


# Statuses that fail a sweep, in the order its summary lists every status.
FAILING = ("fail", "diverged", "timeout", "error")
SUMMARY_ORDER = ("pass", *FAILING, "skip")
# Results between rewrites of the streamed results file.
PROGRESS_EVERY = 200


@dataclass
class Sweep:
    """Runs a process pool works through. Each gives a result dict with
    ``test``, ``status``, ``pipeline`` when there are several, and a
    ``reason`` when it did not pass."""

    title: str
    run_one: Callable[[Any], dict]
    work: list
    out: str
    # The test and pipeline a run is of, to record one that raised.
    describe: Callable[[Any], tuple[str, str | None]]
    jobs: int | None = None
    # Extra top-level fields of the results file, such as the pipelines.
    header: dict = field(default_factory=dict)
    skipped: list = field(default_factory=list)


def run_sweep(sweep):
    """Runs `sweep`, streaming its results to ``sweep.out`` and printing
    each run that did not pass. Returns the exit status: 1 when any failed."""
    results = list(sweep.skipped)
    counts = Counter(result["status"] for result in results)
    os.makedirs(os.path.dirname(sweep.out) or ".", exist_ok=True)
    print(f"streaming results to: {sweep.out}")
    try:
        with cf.ProcessPoolExecutor(max_workers=sweep.jobs) as pool:
            futures = {pool.submit(sweep.run_one, run): run for run in sweep.work}
            for i, future in enumerate(cf.as_completed(futures), 1):
                result = _result_of(future, *sweep.describe(futures[future]))
                results.append(result)
                counts[result["status"]] += 1
                if result["status"] != "pass":
                    _print_result(i, len(sweep.work), result)
                if i % PROGRESS_EVERY == 0:
                    _write_results(sweep, results, counts)
    finally:
        _write_results(sweep, results, counts)
    _print_summary(sweep, counts, len(results) - len(sweep.skipped))
    return 1 if any(counts[status] for status in FAILING) else 0


def _result_of(future, test, pipeline):
    """`future`'s result, or an error result naming what it raised."""
    try:
        return future.result()
    except Exception as error:
        result = {
            "test": test,
            "status": "error",
            "reason": f"{type(error).__name__}: {error}",
        }
        if pipeline is not None:
            result["pipeline"] = pipeline
        return result


def _print_result(i, total, result):
    pipeline = f"{result['pipeline']:24} " if "pipeline" in result else ""
    reason = f": {result['reason'][:120]}" if result.get("reason") else ""
    print(
        f"[{i:6}/{total}] {result['status'].upper():8} {pipeline}{result['test']}{reason}",
        flush=True,
    )


def _write_results(sweep, results, counts):
    with open(sweep.out, "w") as f:
        json.dump(
            {
                **sweep.header,
                "total_planned": len(sweep.work) + len(sweep.skipped),
                "completed": len(results),
                "counts": dict(counts),
                "results": results,
            },
            f,
            indent=2,
        )


def _print_summary(sweep, counts, completed):
    print()
    print(f"=== {sweep.title}: {completed} / {len(sweep.work)} completed ===")
    for status in SUMMARY_ORDER:
        if counts[status]:
            print(f"  {status:9} {counts[status]}")
    print(f"results: {sweep.out}")
