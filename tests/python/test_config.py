"""Tests for the Python Config object.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import inspect
import pickle
import unittest

from rvsim import (
    Cache,
    Config,
    LoadPrefetcher,
    PageBoundary,
    Simulator,
    StorePrefetcher,
    presets,
)


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


class Ownership(unittest.TestCase):
    def test_default_caches_are_not_shared_between_configs(self):
        first = Config()
        first.l1i.latency = 99

        second = Config()

        self.assertNotEqual(second.l1i.latency, 99)

    def test_a_replaced_config_does_not_share_its_caches(self):
        base = Config()
        wide = base.replace(width=8)

        wide.l1d.latency = 7

        self.assertNotEqual(base.l1d.latency, 7)


class Pickle(unittest.TestCase):
    def test_round_trips_a_config_with_vector_units(self):
        config = presets.m1()

        restored = pickle.loads(pickle.dumps(config))

        self.assertEqual(restored.to_dict(), config.to_dict())


if __name__ == "__main__":
    unittest.main()


class Prefetchers(unittest.TestCase):
    def test_the_load_and_store_prefetchers_reach_the_core(self):
        config = Config(
            load_prefetcher=LoadPrefetcher.Stride(
                l1_lines=4, l2_lines=22, page_boundary=PageBoundary.CrossWithTlb()
            ),
            store_prefetcher=StorePrefetcher.Stream(streams=4, l2_lines=8),
        )

        cache = config.to_dict()["cache"]
        Simulator(config)

        self.assertEqual(cache["load_prefetcher"]["page_boundary"], "CrossWithTlb")
        self.assertEqual(
            cache["store_prefetcher"], {"kind": "Stream", "streams": 4, "l2_lines": 8}
        )

    def test_no_prefetcher_by_default(self):
        cache = Config().to_dict()["cache"]

        self.assertEqual(cache["load_prefetcher"], {"kind": "None"})
        self.assertEqual(cache["store_prefetcher"], {"kind": "None"})


class Validation(unittest.TestCase):
    def test_a_vlen_the_vector_unit_cannot_have_is_refused(self):
        for vlen in (0, 64, 100, 4096):
            with self.subTest(vlen=vlen), self.assertRaisesRegex(ValueError, "VLEN"):
                Simulator(Config(vlen=vlen))

    def test_a_power_of_two_vlen_in_range_is_accepted(self):
        Simulator(Config(vlen=256))

    def test_a_cache_with_zero_of_a_resource_is_refused(self):
        for field in ("mshr_count", "write_buffers", "targets_per_mshr"):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, field):
                Cache("32KB", ways=4, **{field: 0})

    def test_a_config_dict_with_zero_mshrs_is_refused(self):
        config = Config().to_dict()
        config["cache"]["l1_d"]["mshr_count"] = 0

        with self.assertRaisesRegex(ValueError, "nonzero"):
            Simulator(config)

    def test_an_unset_resource_count_is_left_to_the_simulator(self):
        l1d = Config(l1d=Cache("32KB", ways=4)).to_dict()["cache"]["l1_d"]

        self.assertNotIn("mshr_count", l1d)
        self.assertNotIn("write_buffers", l1d)
        self.assertNotIn("targets_per_mshr", l1d)

    def test_a_field_the_core_does_not_know_is_refused(self):
        config = Config().to_dict()
        config["pipeline"]["widht"] = 4

        with self.assertRaisesRegex(ValueError, "unknown field `widht`"):
            Simulator(config)
