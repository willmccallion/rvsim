"""Give the ``rvsim._core`` stub the extension's docstrings.

The stub ``rvsim/_core.pyi`` carries the types; the docstrings live in the
Rust sources and reach Python as ``__doc__``. mkdocstrings reads the stub,
so this extension inspects the built extension and copies every docstring
the stub lacks onto the matching stub object.
"""

from __future__ import annotations

import griffe


class CoreDocstrings(griffe.Extension):
    """Copy runtime docstrings onto the ``rvsim._core`` stub objects."""

    def on_package_loaded(self, *, pkg: griffe.Module, **kwargs: object) -> None:
        if pkg.name != "rvsim" or "_core" not in pkg.members:
            return
        runtime = griffe.load("rvsim._core", force_inspection=True)
        _copy_docstrings(runtime, pkg["_core"])


def _copy_docstrings(source: griffe.Object, target: griffe.Object) -> None:
    for name, member in source.members.items():
        if name.startswith("_") or name not in target.members:
            continue
        stub_member = target.members[name]
        if isinstance(member, griffe.Alias) or isinstance(stub_member, griffe.Alias):
            continue
        if member.docstring and not stub_member.docstring:
            stub_member.docstring = griffe.Docstring(
                member.docstring.value, parent=stub_member
            )
        _copy_docstrings(member, stub_member)
