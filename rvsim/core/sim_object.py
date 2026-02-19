"""
Base SimObject for configuration serialization.

Provides to_dict() and to_json() for recursively serializing object configuration to a format
suitable for the Rust backend or logging.
"""

import json


class SimObject:
    """Base class for simulated hardware objects; supports recursive config serialization."""

    def to_dict(self):
        """Recursively convert the object configuration to a dictionary (nested SimObjects become dicts)."""
        config = {}
        for key, value in self.__dict__.items():
            if hasattr(value, "to_dict"):
                config[key] = value.to_dict()
            else:
                config[key] = value
        return config

    def to_json(self):
        """Return the configuration as a JSON string (e.g., for the Rust backend or logging)."""
        return json.dumps(self.to_dict())
