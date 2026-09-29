"""Tests for the Python Config object.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import inspect
import pickle
import unittest

from rvsim import Config, presets


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


class Pickle(unittest.TestCase):
    def test_round_trips_a_config_with_vector_units(self):
        config = presets.m1()

        restored = pickle.loads(pickle.dumps(config))

        self.assertEqual(restored.to_dict(), config.to_dict())


if __name__ == "__main__":
    unittest.main()
