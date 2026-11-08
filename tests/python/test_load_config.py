"""Tests for loading a Config from a Python file.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import os
import tempfile
import unittest

from rvsim import Config
from rvsim.config import load_config


def _write(directory, name, source):
    path = os.path.join(directory, name)
    with open(path, "w") as f:
        f.write(source)
    return path


class LoadConfig(unittest.TestCase):
    def test_a_config_variable_is_loaded(self):
        with tempfile.TemporaryDirectory() as directory:
            path = _write(
                directory,
                "machine.py",
                "from rvsim import Config\nconfig = Config(hart_count=2)\n",
            )

            loaded = load_config(path)

        self.assertIsInstance(loaded, Config)
        self.assertEqual(loaded.hart_count, 2)

    def test_a_function_named_after_the_file_is_called(self):
        with tempfile.TemporaryDirectory() as directory:
            path = _write(
                directory,
                "machine.py",
                "from rvsim import Config\n"
                "def machine():\n    return Config(hart_count=3)\n",
            )

            self.assertEqual(load_config(path).hart_count, 3)

    def test_an_entry_point_giving_something_else_is_a_type_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = _write(directory, "machine.py", "config = {'hart_count': 2}\n")

            with self.assertRaisesRegex(TypeError, "config gave a dict"):
                load_config(path)

    def test_a_file_without_an_entry_point_is_an_attribute_error(self):
        with tempfile.TemporaryDirectory() as directory:
            path = _write(directory, "machine.py", "x = 1\n")

            with self.assertRaises(AttributeError):
                load_config(path)


if __name__ == "__main__":
    unittest.main()
