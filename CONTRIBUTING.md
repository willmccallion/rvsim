# Contributing to rvsim

Thanks for your interest in rvsim. This guide covers how the repository is
laid out, how to build and test it, and what a change needs before it can be
merged.

## Repository layout

| Path | Contents |
|------|----------|
| `crates/rvsim-core/` | The simulator, in Rust. `src/lib.rs` lists its layers in dependency order. |
| `crates/rvsim-bindings/` | The PyO3 bindings that expose the core as `rvsim._core`. |
| `rvsim/` | The Python package: configuration, sessions, presets, the `rvsim` CLI. |
| `software/` | Guest-side libc, linker scripts and the Linux image build. |
| `examples/` | Guest programs and benchmarks, and Python analysis scripts. |
| `tests/` | Python API tests and the ISA conformance runners. See [`tests/README.md`](tests/README.md). |
| `tools/` | Developer tools: the gem5 comparison, baseline recorders, the Linux boot driver. |
| `docs/` | The documentation site, including architecture notes and decision records. |

A module in `rvsim-core` may depend only on the layers listed before it in
`lib.rs`, apart from the one exception described there. Put new code in the
lowest layer that has everything it needs.

## Building

You need a Rust toolchain (edition 2024), Python 3.10 or newer, and a
`riscv64` GCC toolchain for the guest programs. `nix develop` provides all
of them.

```sh
make python      # build the extension into .venv (maturin develop --release)
make software    # build the guest programs into software/bin
```

## Checks

Run these before opening a pull request:

```sh
make fmt-check   # rustfmt and ruff format
make clippy      # clippy, with warnings as errors
make test        # Rust unit and integration tests
make test-python # Python API tests
```

Changes to the ISA, the pipelines or the memory system should also pass the
conformance suites:

```sh
make riscv-tests
make vector-test
make test-all-smoke
```

The workspace denies `clippy::pedantic` and `clippy::nursery`. Fix a lint
rather than allowing it, unless the surrounding code already allows it for
the same reason. Production code does not use `unwrap`, `expect` or `panic!`,
and every `unsafe` block carries a `// SAFETY:` comment naming the invariant
it relies on.

## Timing changes and refactors

A refactor must leave every cycle count unchanged
([ADR 0001](docs/architecture/decisions/0001-refactors-are-cycle-identical.md)).
Record a baseline with `tools/diag/cycle_baseline.py` before the change and
compare against it after. A change that is meant to move timing goes in its
own commit, with a regression test that pins the new behaviour.

Architectural decisions are recorded in
[`docs/architecture/decisions/`](docs/architecture/decisions/index.md). Add a
record when a change settles a question that future contributors would
otherwise reopen.

## Commits and pull requests

- Keep each commit to one logical change, and make sure it builds and passes
  the checks on its own.
- Write the subject in the imperative mood, at most 72 characters, with no
  trailing period. Add a body only when the reason for the change is not
  obvious from the diff.
- Describe what the code does. Leave out plans, milestones and task numbers.
- Update the documentation and [`docs/changelog.md`](docs/changelog.md) for
  user-visible changes.

## AI-assisted contributions

You may use AI tools to write code, tests or documentation. Every AI-assisted
change must be reviewed and approved by a human maintainer before it is
merged. The person who submits the change is responsible for it: they must
understand the code, be able to explain and defend it in review, and have
run the checks above themselves. Don't submit code that you couldn't have
written or debugged yourself.

## License

By contributing, you agree that your contributions are licensed under the
same terms as the project, [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option.
