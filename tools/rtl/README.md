# RTL comparison

Runs the same ELFs on rvsim and on Chipyard's Rocket and BOOM, built as
Verilator simulators, and lines the retire traces up instruction by
instruction. The results and their causes are in `docs/rtl-rocket.md` and
`docs/rtl-boom.md`.

## Usage

```bash
nix develop .#rtl                                  # JDK, firtool 1.75.0, Verilator, RISCV
make rtl-build                                     # one-time, about an hour
bash tools/rtl/programs/build.sh                   # -> tests/builds/rtl-programs/*.elf
python tools/diag/rtl_compare.py --core rocket     # or --core boom
python tools/diag/rtl_probe.py --core rocket       # the latencies the presets cite
python tools/diag/rtl_probe.py --core rocket --config rocket   # the same on rvsim
```

`make rtl-compare RTL_CORE=rocket` does the last three builds and the
comparison; `rtl_compare.py --config basic` compares against another preset.

## Layout

- `build.sh` clones Chipyard at `CHIPYARD_REV` from
  `tests/conformance/sources.mk` into `tests/builds/chipyard`, initialises
  its minimal submodule set, applies `patches/`, copies `RvsimConfigs.scala`
  into Chipyard's config package and builds each configuration's simulator.
- `patches/boom-commit-log-cycle.patch` adds BOOM's cycle counter to its
  commit-log printf; Rocket's `+verbose` trace already carries one.
- `RvsimConfigs.scala` defines `RvsimMediumBoomV4Config`: `MediumBoomV4Config`
  with the commit log on. Rocket runs as `RocketConfig`.
- `programs/` holds the one-mechanism kernels, `crt0.s` (sets the stack,
  enables the FPU, exits through HTIF so the same ELF runs on both sides),
  `rtl.ld` and the build script, which also builds the scalar kernels of
  `tools/gem5_compare/programs`.

## Running a simulator by hand

```bash
tests/builds/chipyard/sims/verilator/simulator-chipyard.harness-RocketConfig \
    +permissive +verbose +max-cycles=50000000 +loadmem=prog.elf +permissive-off prog.elf \
    2> trace.txt
```

The trace and the harness's `*** PASSED ***` go to stderr. `+loadmem` puts
the ELF straight into the simulated DRAM; without it the harness streams it
over the serial link and a program with a large `.bss` never starts.
Memory is Chipyard's zero-latency model; `+dramsim` would put DRAMSim2 in
its place.
