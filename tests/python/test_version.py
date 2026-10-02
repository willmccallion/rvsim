"""The native extension reports the version it was built as."""

import unittest

import rvsim
from rvsim import _core


class Version(unittest.TestCase):
    def test_the_extension_and_the_package_agree(self):
        self.assertEqual(_core.version(), rvsim.__version__)
