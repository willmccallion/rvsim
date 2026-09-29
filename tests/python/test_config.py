"""Tests for the Python Config object.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import inspect
import unittest

from rvsim import Config


class Replace(unittest.TestCase):
    def test_accepts_every_constructor_parameter(self):
        base = Config()
        names = [
            n for n in inspect.signature(Config.__init__).parameters if n != "self"
        ]

        for name in names:
            with self.subTest(name=name):
                copy = base.replace(**{name: getattr(base, name)})
                self.assertEqual(copy.to_dict(), base.to_dict())

    def test_rejects_an_unknown_field(self):
        with self.assertRaises(TypeError):
            Config().replace(not_a_field=1)


if __name__ == "__main__":
    unittest.main()
