"""The cache of fast-forward checkpoints, keyed by everything that led to them."""

from __future__ import annotations

import json
import os
from typing import Any, Dict

from .. import _core


_META_FORMAT = 1


def default_cache_dir() -> str:
    """``$RVSIM_CACHE_DIR``, else ``rvsim/checkpoints`` under the user's
    cache directory."""
    explicit = os.environ.get("RVSIM_CACHE_DIR")
    if explicit:
        return explicit
    base = os.environ.get("XDG_CACHE_HOME") or os.path.join(os.path.expanduser("~"), ".cache")
    return os.path.join(base, "rvsim", "checkpoints")


class _Cache:
    """Checkpoints named by key: ``<key>.ckpt`` beside ``<key>.ckpt.json``."""

    def __init__(self, directory: str):
        self.directory = directory

    def path(self, key: str) -> str:
        return os.path.join(self.directory, key + ".ckpt")

    def has(self, key: str) -> bool:
        path = self.path(key)
        if not (os.path.exists(path) and os.path.exists(path + ".json")):
            return False
        return _read_meta(path)["checkpoint_version"] == _core.CHECKPOINT_VERSION


def _read_meta(path: str) -> Dict[str, Any]:
    with open(path + ".json") as f:
        meta = json.load(f)
    if meta.get("format") != _META_FORMAT:
        raise ValueError(f"{path}.json is not a session checkpoint this rvsim reads")
    return meta
