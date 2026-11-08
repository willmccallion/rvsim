"""Where the guest's console connects is one setting, a `ConsoleMode`,
which also takes its value as a string.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import unittest

from rvsim import Config, ConsoleMode


class ConsoleSetting(unittest.TestCase):
    def test_the_default_console_is_stdout(self):
        self.assertIs(Config().console, ConsoleMode.STDOUT)

    def test_a_mode_named_by_its_string_is_the_same_mode(self):
        self.assertIs(Config(console="Quiet").console, ConsoleMode.QUIET)

    def test_the_mode_reaches_the_core_config_by_name(self):
        config = Config(console=ConsoleMode.CAPTURED).to_dict()

        self.assertEqual(config["system"]["console"], "Captured")

    def test_an_unknown_mode_is_refused(self):
        with self.assertRaisesRegex(ValueError, "'Silent' is not a valid ConsoleMode"):
            Config(console="Silent")

    def test_replace_sets_the_mode(self):
        quiet = Config().replace(console=ConsoleMode.QUIET)

        self.assertIs(quiet.console, ConsoleMode.QUIET)


if __name__ == "__main__":
    unittest.main()
