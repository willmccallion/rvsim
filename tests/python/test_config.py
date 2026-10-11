"""Tests for the Python Config object.

Run with: .venv/bin/python -m unittest discover -s tests/python
"""

import inspect
import pickle
import unittest

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    LoadPrefetcher,
    PageBoundary,
    Simulator,
    StorePrefetcher,
    presets,
)
from rvsim._core import check_config, default_config, minimal_config
from tests.conformance.configs.pipelines import PIPELINES
from tools.diag.calibration import CALIBRATED


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
        config = presets.fast()

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

    def test_the_bht_predictor_reaches_the_core(self):
        config = Config(
            branch_predictor=BranchPredictor.Bht(
                entries=1024, counter_bits=2, history_length=10, history_bits=4
            )
        )

        pipeline = config.to_dict()["pipeline"]
        Simulator(config)

        self.assertEqual(pipeline["branch_predictor"], "Bht")
        self.assertEqual(
            pipeline["bht"],
            {
                "entries": 1024,
                "counter_bits": 2,
                "history_length": 10,
                "history_bits": 4,
            },
        )

    def test_no_prefetcher_by_default(self):
        cache = Config().to_dict()["cache"]

        self.assertEqual(cache["load_prefetcher"], {"kind": "None"})
        self.assertEqual(cache["store_prefetcher"], {"kind": "None"})


class RustMachines(unittest.TestCase):
    def test_config_is_the_cores_default_machine(self):
        self.assertEqual(check_config(Config().to_dict()), default_config())

    def test_the_minimal_machine_has_no_caches(self):
        cache = minimal_config()["cache"]

        enabled = [cache[level]["enabled"] for level in ("l1_i", "l1_d", "l2", "l3")]

        self.assertEqual(enabled, [False] * 4)

    def test_every_built_in_configuration_passes_the_cores_checks(self):
        configs = {
            "Config()": Config(),
            "linux(harts=2)": presets.linux(harts=2),
            **{f"preset {name}": make() for name, make in presets.PRESETS.items()},
            **{f"calibrated {name}": make() for name, make in CALIBRATED.items()},
            **{f"matrix {label}": config for label, config in PIPELINES},
        }

        for name, config in configs.items():
            with self.subTest(config=name):
                check_config(config.to_dict())

    def test_a_machine_that_would_hang_is_refused_when_built(self):
        with self.assertRaisesRegex(ValueError, "pipeline.width must be at least 1"):
            Simulator(Config(width=0))

    def test_a_machine_the_core_refuses_raises_value_error(self):
        config = Config(btb_size=96, btb_ways=4)

        with self.assertRaisesRegex(ValueError, "btb_size"):
            check_config(config.to_dict())


class Validation(unittest.TestCase):
    def test_a_vlen_the_vector_unit_cannot_have_is_refused(self):
        for vlen in (0, 64, 100, 4096):
            with self.subTest(vlen=vlen), self.assertRaisesRegex(ValueError, "VLEN"):
                Simulator(Config(vlen=vlen))

    def test_a_power_of_two_vlen_in_range_is_accepted(self):
        Simulator(Config(vlen=256))

    def test_never_squashing_after_a_csr_needs_the_out_of_order_backend(self):
        with self.assertRaisesRegex(ValueError, "csr_squash"):
            Config(backend=Backend.InOrder(), csr_squash="Never")
        Config(backend=Backend.OutOfOrder(), csr_squash="Never")

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

    def test_an_isa_string_reaches_the_core(self):
        config = Config(isa="RV64IM")

        self.assertEqual(config.to_dict()["isa"]["string"], "RV64IM")
        Simulator(config)

    def test_the_isa_string_resolves_to_what_the_hart_implements(self):
        everything = Config().isa_string()
        named = Config(isa="rv64gc_zba").isa_string()

        self.assertTrue(everything.startswith("rv64imafdcbv_zicbom_"), everything)
        self.assertIn("_zvkned_", everything)
        self.assertEqual(
            named, "rv64imafdc_zicntr_zicsr_zifencei_zihintpause_zihpm_zba_sstc"
        )

    def test_an_unknown_extension_name_is_refused(self):
        with self.assertRaisesRegex(ValueError, "zxyz"):
            Config(isa="rv64gc_zxyz").isa_string()
        with self.assertRaisesRegex(ValueError, "zxyz"):
            Simulator(Config(isa="rv64gc_zxyz"))

    def test_an_isa_the_hart_cannot_implement_is_refused(self):
        with self.assertRaisesRegex(ValueError, "RV64"):
            Simulator(Config(isa="RV32I"))

    def test_a_field_the_core_does_not_know_is_refused(self):
        config = Config().to_dict()
        config["pipeline"]["widht"] = 4

        with self.assertRaisesRegex(ValueError, "unknown field `widht`"):
            Simulator(config)


class InOrderBackend(unittest.TestCase):
    def test_its_unit_pool_reaches_the_core(self):
        pool = Fu([Fu.IntAlu(count=1, latency=1), Fu.IntMul(count=1, latency=8)])

        config = Config(backend=Backend.InOrder(fu_config=pool)).to_dict()

        self.assertEqual(config["pipeline"]["fu_config"]["num_int_mul"], 1)
        self.assertEqual(config["pipeline"]["fu_config"]["int_mul_latency"], 8)
        self.assertEqual(config["pipeline"]["fu_config"]["num_fp_add"], 0)
