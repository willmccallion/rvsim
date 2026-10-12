"""A pipeline trace is a Kanata log Konata reads, taken from Python around
part of a run or from the command line for a whole one.
"""

import os
import subprocess
import sys
import tempfile
import unittest

from rvsim import Config, Simulator

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")


@unittest.skipUnless(os.path.exists(FIB), "fib.elf not built")
class PipelineTrace(unittest.TestCase):
    def test_a_trace_covers_the_part_of_the_run_it_is_open_for(self):
        cpu = Simulator(Config(console="Quiet"), binary=FIB)
        cpu.run(limit=2_000)
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "fib.kanata")
            cpu.open_pipeline_trace(path)
            cpu.run(limit=20_000)
            cpu.close_pipeline_trace()
            with open(path) as f:
                lines = f.read().splitlines()

        self.assertEqual(lines[0], "Kanata\t0004")
        self.assertTrue(lines[1].startswith("C=\t"), lines[1])
        first_cycle = int(lines[1].split("\t")[1])
        self.assertGreaterEqual(first_cycle, 2_000)
        self.assertTrue(any(line.startswith("R\t") for line in lines))

    def test_an_unknown_format_is_refused(self):
        cpu = Simulator(Config(console="Quiet"), binary=FIB)

        with self.assertRaises(ValueError):
            cpu.open_pipeline_trace("unused.trace", format="o3pipeview")

    def test_the_command_line_writes_the_whole_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "fib.kanata")
            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "rvsim",
                    FIB,
                    "--no-stats",
                    "--pipeline-trace",
                    path,
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            with open(path) as f:
                log = f.read()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(log.startswith("Kanata\t0004\nC=\t"))
        retired = sum(
            1
            for line in log.splitlines()
            if line.endswith("\t0") and line.startswith("R\t")
        )
        self.assertGreater(retired, 100)


if __name__ == "__main__":
    unittest.main()
