"""The core's default machine and each configuration component's default,
read once from the native module. Every default of ``Config`` and of the
component builders comes from here, so the core is the one place they are
written."""

from __future__ import annotations

from typing import Any

from .._core import component_defaults, default_config

#: The default machine, as the nested dict a simulator takes.
MACHINE: dict[str, Any] = default_config()

#: Each component at its own default, by name.
COMPONENTS: dict[str, Any] = component_defaults()
