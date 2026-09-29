"""Session runs a workload in phases: stop points, cached fast-forwards,
configuration switches, forks, saved resume points and measured regions.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import tempfile
import unittest

from rvsim import (
    LOGIN_SHELL,
    AnyOf,
    Backend,
    Config,
    Console,
    Cycles,
    Exit,
    Instructions,
    Marker,
    Pc,
    Session,
    When,
    WorkloadEnded,
)
from rvsim.session.session import _typed_command_output

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
REGIONS = os.path.join(ROOT, "software", "bin", "programs", "regions.elf")
FIB = os.path.join(ROOT, "software", "bin", "programs", "fib.elf")

RESUME_POINT = 1
WORK_START, WORK_END = 10, 11
SETUP_LINE = "setup 2664667000\n"
WORK_LINE = "work 2666466670000\n"


def in_order() -> Config:
    return Config(backend=Backend.InOrder(), width=1)


class SessionTestCase(unittest.TestCase):
    def setUp(self):
        self._cache = tempfile.TemporaryDirectory()
        self.cache_dir = self._cache.name

    def tearDown(self):
        self._cache.cleanup()

    def session(self, config=None, binary=REGIONS, **kwargs) -> Session:
        return Session(
            config or Config(), binary=binary, cache_dir=self.cache_dir, **kwargs
        )


@unittest.skipUnless(os.path.exists(REGIONS), "regions.elf not built")
class StopPoints(SessionTestCase):
    def test_a_marker_stops_at_the_guest_break(self):
        s = self.session()

        stopped = s.run(Marker(RESUME_POINT))

        self.assertIs(type(stopped.by), Marker)
        self.assertEqual(stopped.label, RESUME_POINT)
        self.assertEqual(s.console, SETUP_LINE)

    def test_a_marker_for_another_label_lets_the_run_continue(self):
        s = self.session()

        stopped = s.run(Marker(99))

        self.assertTrue(stopped.exited)
        self.assertEqual(stopped.exit_code, 0)
        self.assertEqual(s.console, SETUP_LINE + WORK_LINE)

    def test_a_console_pattern_stops_on_the_cycle_the_output_appears(self):
        s = self.session()

        stopped = s.run(Console(r"setup \d+\n"))

        self.assertEqual(stopped.match.group(0), SETUP_LINE)
        self.assertEqual(
            s.console, SETUP_LINE, "nothing after the line has been printed"
        )

    def test_a_console_match_consumes_the_output_it_matched(self):
        s = self.session()
        s.run(Console("setup"))

        stopped = s.run(Console("setup") | Exit())

        self.assertTrue(
            stopped.exited, "the second wait does not match the first line again"
        )

    def test_cycle_and_instruction_counts_are_relative_to_the_run_start(self):
        s = self.session()
        s.run(Cycles(1000))

        by_cycles = s.run(Cycles(500))
        start = s.instructions
        by_instructions = s.run(Instructions(700))

        self.assertEqual(by_cycles.cycle, 1500)
        self.assertGreaterEqual(by_instructions.instructions - start, 700)
        self.assertLess(by_instructions.instructions - start, 700 + 8)

    def test_whichever_stop_holds_first_ends_the_run(self):
        s = self.session()

        stopped = s.run(Cycles(1_000_000) | Marker(RESUME_POINT))

        self.assertIs(type(stopped.by), Marker)

    def test_a_pc_stop_reports_the_hart(self):
        s = self.session()
        entry = s.sim.pc

        stopped = s.run(Pc(0xDEAD_0000, entry + 8))

        self.assertEqual(stopped.hart, 0)
        self.assertEqual(s.sim.pc, entry + 8)

    def test_a_predicate_is_checked_every_so_many_cycles(self):
        s = self.session()

        stopped = s.run(When(lambda session: session.cycle >= 3000, every=1000))

        self.assertIs(type(stopped.by), When)
        self.assertEqual(stopped.cycle, 3000)

    def test_a_periodic_callback_sees_every_interval(self):
        s = self.session()
        seen = []

        s.run(
            Marker(RESUME_POINT),
            every=1000,
            on_every=lambda session: seen.append(session.cycle),
        )

        self.assertEqual(seen, [1000 * (i + 1) for i in range(len(seen))])
        self.assertGreaterEqual(len(seen), 3)

    def test_a_login_shell_cannot_be_combined(self):
        with self.assertRaises(TypeError):
            AnyOf(LOGIN_SHELL, Cycles(10))._primitives()


@unittest.skipUnless(os.path.exists(REGIONS), "regions.elf not built")
class FastForwardCache(SessionTestCase):
    def test_the_second_fast_forward_restores_the_first_ones_checkpoint(self):
        first = self.session().fast_forward(Marker(RESUME_POINT))
        second = self.session().fast_forward(Marker(RESUME_POINT))

        self.assertFalse(first.last_fast_forward.cached)
        self.assertTrue(second.last_fast_forward.cached)
        self.assertEqual(second.console, SETUP_LINE)
        self.assertEqual(second.cycle, first.cycle)

    def test_a_restored_fast_forward_finishes_exactly_as_the_run_it_skipped(self):
        uncached = self.session().fast_forward(Marker(RESUME_POINT), cache=False)
        uncached.run()
        cached = self.session().fast_forward(Marker(RESUME_POINT))
        cached.run()

        self.assertEqual(cached.console, uncached.console)
        self.assertEqual(cached.instructions, uncached.instructions)
        self.assertEqual(cached.cycle, uncached.cycle)

    def test_a_different_history_misses_the_cache(self):
        self.session().fast_forward(Marker(RESUME_POINT))
        s = self.session()
        s.run(Cycles(100))

        s.fast_forward(Marker(RESUME_POINT))

        self.assertFalse(s.last_fast_forward.cached)

    def test_the_fast_forward_config_is_part_of_the_key_and_the_session_config_is_not(
        self,
    ):
        self.session(fast_forward_config=in_order()).fast_forward(Marker(RESUME_POINT))
        other_core = self.session(Config(width=2), fast_forward_config=in_order())
        other_fast_forward = self.session(fast_forward_config=Config(width=2))

        other_core.fast_forward(Marker(RESUME_POINT))
        other_fast_forward.fast_forward(Marker(RESUME_POINT))

        self.assertTrue(other_core.last_fast_forward.cached)
        self.assertFalse(other_fast_forward.last_fast_forward.cached)

    def test_a_fast_forward_continues_on_the_session_config(self):
        wide = self.session(Config(width=4), fast_forward_config=in_order())
        narrow = self.session(in_order(), fast_forward_config=in_order())

        wide_work = wide.fast_forward(Marker(RESUME_POINT)).measure(until=Exit())
        narrow_work = narrow.fast_forward(Marker(RESUME_POINT)).measure(until=Exit())

        self.assertTrue(narrow.last_fast_forward.cached)
        self.assertEqual(wide_work.instructions, narrow_work.instructions)
        self.assertLess(wide_work.cycles, narrow_work.cycles)

    def test_a_predicate_without_a_name_cannot_be_cached(self):
        s = self.session()

        with self.assertRaises(ValueError):
            s.fast_forward(When(lambda session: True))
        s.fast_forward(
            When(lambda session: session.cycle >= 2000, every=1000), cache=False
        )

    def test_an_unnamed_predicate_in_the_history_stops_later_caching(self):
        s = self.session()
        s.run(When(lambda session: session.cycle >= 1000, every=1000))

        with self.assertRaises(ValueError):
            s.fast_forward(Marker(RESUME_POINT))

    def test_a_workload_that_ends_first_raises(self):
        s = self.session()

        with self.assertRaises(WorkloadEnded):
            s.fast_forward(Marker(99))


@unittest.skipUnless(os.path.exists(REGIONS), "regions.elf not built")
class SwitchForkResume(SessionTestCase):
    def test_a_switch_carries_the_run_onto_the_new_config(self):
        s = self.session(in_order())
        s.run(Marker(RESUME_POINT))

        s.switch(Config(width=4))
        stopped = s.run()

        self.assertEqual(stopped.exit_code, 0)
        self.assertEqual(s.console, SETUP_LINE + WORK_LINE)

    def test_a_switch_that_changes_the_system_the_guest_sees_is_refused(self):
        s = self.session()
        s.run(Cycles(100))

        with self.assertRaisesRegex(ValueError, "ram_size"):
            s.switch(Config(ram_size="128MB"))

    def test_forks_continue_independently_from_the_same_point(self):
        s = self.session(in_order())
        s.run(Marker(RESUME_POINT))
        point = s.instructions

        results = {
            name: (child.measure(until=Exit()), child.console)
            for name, child in s.fork({"narrow": in_order(), "wide": Config(width=4)})
        }

        self.assertEqual(s.instructions, point, "the parent stays where it was")
        for region, console in results.values():
            self.assertEqual(region.exit_code, 0)
            self.assertEqual(console, SETUP_LINE + WORK_LINE)
        self.assertLess(results["wide"][0].cycles, results["narrow"][0].cycles)

    def test_a_saved_session_resumes_with_its_console_and_config(self):
        s = self.session(in_order())
        s.run(Marker(RESUME_POINT))
        path = s.save(os.path.join(self.cache_dir, "point.ckpt"))

        resumed = Session.resume(path, cache_dir=self.cache_dir)
        resumed.run()
        s.run()

        self.assertEqual(resumed.console, SETUP_LINE + WORK_LINE)
        self.assertEqual(resumed.instructions, s.instructions)

    def test_a_resume_onto_a_changed_workload_is_refused(self):
        s = self.session()
        s.run(Cycles(100))
        path = s.save(os.path.join(self.cache_dir, "point.ckpt"))
        with open(path + ".json") as f:
            text = f.read()
        with open(path + ".json", "w") as f:
            f.write(text.replace(REGIONS, FIB))

        with self.assertRaises(ValueError):
            Session.resume(path, cache_dir=self.cache_dir)


@unittest.skipUnless(os.path.exists(REGIONS), "regions.elf not built")
class Measurement(SessionTestCase):
    def test_a_measured_run_has_only_its_own_stats(self):
        s = self.session()
        s.run(Marker(RESUME_POINT))
        before = s.stats

        region = s.measure(until=Exit())

        self.assertEqual(region.exit_code, 0)
        self.assertEqual(region.console, WORK_LINE)
        self.assertEqual(region.cycles, s.stats.cycles - before.cycles)
        self.assertEqual(
            region.instructions,
            s.stats.instructions_retired - before.instructions_retired,
        )

    def test_the_guest_marked_region_matches_its_snapshots(self):
        s = self.session()
        s.run()

        region = s.sim.stats_between(WORK_START, WORK_END)

        self.assertGreater(region.instructions_retired, 40_000)

    def test_commands_need_a_linux_shell(self):
        s = self.session()

        with self.assertRaises(ValueError):
            s.measure("true")

    def test_a_region_serializes_every_stat(self):
        s = self.session()

        record = s.measure(until=Marker(RESUME_POINT)).to_dict()

        self.assertIn("core0.commit.op.alu", record["stats"])
        self.assertEqual(record["console"], SETUP_LINE)


class TypedCommandOutput(unittest.TestCase):
    def test_the_echoed_command_line_is_dropped_even_when_the_terminal_wrapped_it(self):
        console = "rvsim run 1099511627776 1099511627777 dhrystone; echo __rvsim_109951162\r\n7776__ $?\r\nDhrystone\r\n"

        self.assertEqual(_typed_command_output(console), "Dhrystone\r\n")

    def test_nothing_was_printed_before_the_echo_finished(self):
        self.assertEqual(_typed_command_output("echo __rvsim_1__ "), "")


if __name__ == "__main__":
    unittest.main()
