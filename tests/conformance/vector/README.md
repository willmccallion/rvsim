# Vector (RVV 1.0) Tests — cosim against spike

Tests rvsim's Vector Processing Unit by running each chipsalliance/riscv-vector-tests
ELF on both rvsim and a local spike build, then diffing the post-execution
memory signature region. Identical signature → pass.

The chipsalliance generator emits tests in the `riscv-test-env` `p`-mode
format, which places the symbols `begin_signature`/`end_signature` around the
test data, so a run on each simulator and a diff of that region is the whole
check.

## One-shot from the repo root

```sh
make vector-test-smoke   # a sample of every instruction class, < 1 min
make vector-test         # full chipsalliance suite (hundreds of tests)
```

The first run will:
1. Clone and build spike from source into `tests/builds/spike-install/`
   (one-time, ~2 min — Arch's `spike` 1.1.0 is too old for modern Z-extensions).
2. Clone the chipsalliance generator into `tests/builds/riscv-vector-tests/`.
3. Build the Go generator (one-time).
4. Generate stage1 .S files filtered by `VECTOR_PATTERN` (default `.*`).
5. Strip the chipsalliance "magic" custom-0 instructions
   (`.word 0x...b`) — these would call into pspike to embed expected values;
   we skip that and rely on cosim diff instead.
6. Compile each test ELF with `riscv64-elf-gcc`.
7. Run each ELF on rvsim and on local spike, dump signature, diff.

## Knobs

```sh
make vector-test VECTOR_VLEN=256
make vector-test VECTOR_PATTERN='^vfadd\.'    # only float adds
```

`VECTOR_VLEN` (default 128) is a power of two in [128, 2048] — must match a
VLEN your rvsim build supports. `VECTOR_PATTERN` is a Go regex applied to
instruction names by the chipsalliance generator.

## Triaging a failure

```sh
.venv/bin/python tests/conformance/vector/triage.py \
    tests/builds/vector/vlen128/vadd_vv-0.elf
```

Prints the first byte of divergence in the signature region, hex windows of
both runs, and the last 40 vector commits from spike's log so you can see
which instruction produced the bad result.

## File map

| File                  | Role                                                     |
|-----------------------|----------------------------------------------------------|
| `build_tests.sh`      | Generate + compile chipsalliance tests under `build/`    |
| `run_vector_tests.py` | Parallel rvsim+spike runner; emits `tests/builds/results/results-vlen{N}.json` |
| `rvsim_run.py`        | Runs one ELF on rvsim and writes its signature region    |
| `triage.py`           | Single-ELF rvsim↔spike diff helper                        |

The generator, spike, the generated `.S` files, the ELFs and the results all
live under `tests/builds/`, which git ignores.

## Coverage caveat

The chipsalliance tests store their per-case result into `resultdata` *before*
the magic custom-0 instruction we strip. We therefore catch any bug where the
final stored vector value differs from spike. Things we *don't* catch with
this flow:

- `vxsat` (saturation flag) divergences not reflected in the stored value.
- Tests that rely on pspike-embedded immediate-value `TEST_CASE` checks for
  intermediate state (rare in the integer/load-store/permute corpus, more
  relevant for the float/crypto extensions).

If those gaps matter later, the next step is building pspike against the local
spike-install (the libs are now available since we build spike from source).
