# Error against Rocket

How far rvsim's retire timing is from Rocket's, instruction by
instruction, on the same programs. Rocket is the in-order core SiFive's
U-series grew from; its RTL is built as a Verilator simulator from
Chipyard, so every cycle difference on a matched configuration is an
rvsim bug, a missing model feature, or a documented decision
([decision 12](architecture/decisions/0012-the-model-follows-real-cores.md)).
The out-of-order counterpart is [Error against BOOM](rtl-boom.md); the
gem5 comparison is [Error against gem5](error.md).

## Method

`make rtl-build` builds Chipyard 1.14.0's `RocketConfig` as a Verilator
simulator under `tests/builds/chipyard` (`tools/rtl/build.sh`, pinned in
`tests/conformance/sources.mk`). `make rtl-compare RTL_CORE=rocket`
(`tools/diag/rtl_compare.py`) runs each program in
`tests/builds/rtl-programs` on it with its retire trace on (`+verbose`:
one line per retired instruction with the cycle it retired in) and on
rvsim's `rocket` preset with the commit log open, which carries the same
cycle. The two traces are lined up by retired PC from the ELF's entry to
the spin after its `tohost` write, so the boot ROM on one side and the
exit on both are left out.

Per program the tool reports the cycle ratio (rvsim over Rocket), the
first instruction whose retire-cycle difference reaches a threshold (50
cycles), and the PCs where most of the difference accumulates, which is
what points at a mechanism.

The programs are the scalar kernels of the gem5 comparison
(`tools/gem5_compare/programs`, without the vector ones: Rocket has no
vector unit) and one-mechanism probes in `tools/rtl/programs`, built for
`rv64gc_zba_zbb_zbs` with a start-up that sets the stack, switches the FPU
on and exits through HTIF, so one ELF runs unchanged on both. The probes
isolate the load-to-use latency of each cache level (`chase_l1`,
`chase_l2`, `chase_mem`: eight dependent loads unrolled, since a short
loop hides a 1-wide core's latency), the multiplier (`mul_chain`), a
CSR access (`csr_serialize`, with `csr_flush` for a write the core
flushes after and `csr_dependent` for a read whose value the next
instruction uses), `FENCE` and `FENCE.I`, a trap and its return
(`trap_return`), a taken branch every few instructions (`branch_taken`)
and a store to a new cache line every iteration (`store_stream`).

Memory behind the L2 is Chipyard's zero-latency model (no `+dramsim`), so
a miss costs the L2 and the TileLink and AXI fabric alone, and rvsim's
`rocket` preset gives its fixed-latency memory the latency measured on
`chase_mem` (#204 tracks a DRAM model both can share).

## Machine

Rocket as `RocketConfig` builds it (`freechips.rocketchip.rocket.WithNHugeCores`
in `rocket/Configs.scala`, `chipyard.config.AbstractConfig`):

- One in-order core with a 5-stage pipeline, 1-wide; `MulDivParams(mulUnroll
  = 8, mulEarlyOut, divEarlyOut)`; an FPU with `minFLen = 16` and the
  default `FPUParams` latencies (`sfmaLatency = 3`, `dfmaLatency = 4`);
  Zba, Zbb and Zbs.
- 32 KiB 8-way L1I (`ICacheParams(nSets = 64, nWays = 8)`, `latency = 2`)
  and 32 KiB 8-way L1D (`DCacheParams(nSets = 64, nWays = 8, nMSHRs = 0)`:
  a blocking cache), 64-byte lines.
- A 28-entry BTB with a 6-entry return address stack and a 512-entry BHT
  with 8 bits of global history (`BTBParams`, `BHTParams` in
  `rocket/BTB.scala`).
- 32-entry fully associative L1 TLBs and a 512-entry L2 TLB
  (`DCacheParams.nTLBWays`, `ICacheParams.nTLBWays`,
  `RocketCoreParams.nL2TLBEntries`).
- 512 KiB 8-way inclusive L2 (`WithInclusiveCache` defaults).

Misaligned accesses trap. The rvsim preset is `presets.rocket()`; each
parameter there cites the line it comes from or the probe it was measured
with.

## Results

Chipyard 1.14.0's `RocketConfig` against `presets.rocket()`, after #210,
#211, #212, #213, #216 and #222. The error is rvsim's cycles over
Rocket's, less one; the last column is the PC where most of the
difference accumulates, as `tools/diag/rtl_compare.py` reports it.

| program | instructions | Rocket cycles | rvsim cycles | error | Rocket IPC | rvsim IPC | where the difference accumulates |
|---|---:|---:|---:|---:|---:|---:|---|
| `alu_int_div` | 40,042 | 104,194 | 104,119 | -0.1% | 0.38 | 0.38 | `main+0x2a` |
| `alu_int_mul` | 160,043 | 480,192 | 480,120 | -0.0% | 0.33 | 0.33 | `main+0x28` |
| `bitmanip` | 1,146,949 | 1,668,741 | 1,534,229 | -8.1% | 0.69 | 0.75 | `main+0x4c` |
| `br_nested_loops` | 196,037 | 236,204 | 214,134 | -9.3% | 0.83 | 0.92 | `main+0x1a` |
| `br_pattern` | 1,005,760 | 1,414,507 | 1,868,713 | +32.1% | 0.71 | 0.54 | `main+0x4a` |
| `br_random` | 280,053 | 700,282 | 700,221 | -0.0% | 0.40 | 0.40 | `main+0x4c` |
| `branch_taken` | 690,037 | 750,192 | 720,131 | -4.0% | 0.92 | 0.96 | `main+0x12` |
| `call_return` | 682,020 | 738,542 | 738,330 | -0.0% | 0.92 | 0.92 | `fib+0x16` |
| `chase_l1` | 83,570 | 135,630 | 135,746 | +0.1% | 0.62 | 0.62 | `main+0xbc` |
| `chase_l2` | 265,331 | 1,277,340 | 1,126,454 | -11.8% | 0.21 | 0.24 | `main+0x72` |
| `chase_mem` | 3,624,529 | 13,835,545 | 11,396,951 | -17.6% | 0.26 | 0.32 | `main+0x72` |
| `csr_dependent` | 100,036 | 140,185 | 180,095 | +28.5% | 0.71 | 0.56 | `main+0x14` |
| `csr_flush` | 60,037 | 160,181 | 220,116 | +37.4% | 0.37 | 0.27 | `main+0x12` |
| `csr_serialize` | 100,037 | 140,186 | 160,096 | +14.2% | 0.71 | 0.62 | `main+0x1c` |
| `div_chain` | 32,040 | 432,253 | 432,124 | -0.0% | 0.07 | 0.07 | `main+0x24` |
| `fence` | 160,232 | 180,560 | 240,329 | +33.1% | 0.89 | 0.67 | `main+0x2c` |
| `fence_i` | 60,037 | 480,182 | 540,142 | +12.5% | 0.13 | 0.11 | `main+0x12` |
| `fp_add_chain` | 120,040 | 240,167 | 240,109 | -0.0% | 0.50 | 0.50 | `main+0x24` |
| `fp_div_chain` | 24,040 | 456,139 | 456,109 | -0.0% | 0.05 | 0.05 | `main+0x20` |
| `fp_fma_parallel` | 120,050 | 120,237 | 120,170 | -0.1% | 1.00 | 1.00 | `main+0x64` |
| `indirect_calls` | 540,049 | 780,254 | 630,206 | -19.2% | 0.69 | 0.86 | `f0+0x8` |
| `l2_pointer_chase` | 782,637 | 3,306,783 | 3,619,949 | +9.5% | 0.24 | 0.22 | `main+0x64` |
| `load_use` | 242,090 | 243,184 | 242,329 | -0.4% | 1.00 | 1.00 | `bss_clear_loop+0x0` |
| `matmul` | 722,021 | 920,512 | 758,300 | -17.6% | 0.78 | 0.95 | `main+0x9e` |
| `mem_conflict` | 356,616 | 1,440,860 | 1,409,816 | -2.2% | 0.25 | 0.25 | `main+0x1e` |
| `mem_dependent_miss` | 1,128,619 | 2,685,777 | 1,588,682 | -40.8% | 0.42 | 0.71 | `main+0x24` |
| `mem_linear` | 983,083 | 1,900,687 | 1,286,565 | -32.3% | 0.52 | 0.76 | `main+0x16` |
| `mem_pointer_chase` | 2,650,413 | 12,321,022 | 13,513,417 | +9.7% | 0.22 | 0.20 | `main+0x68` |
| `mem_random_swap` | 1,179,693 | 5,969,120 | 6,463,243 | +8.3% | 0.20 | 0.18 | `main+0x62` |
| `mem_stride` | 1,613,888 | 4,430,018 | 2,408,646 | -45.6% | 0.36 | 0.67 | `bss_clear_loop+0x0` |
| `mem_write_stream` | 1,441,835 | 3,346,338 | 1,589,446 | -52.5% | 0.43 | 0.91 | `main+0x18` |
| `mul_chain` | 50,042 | 400,162 | 400,100 | -0.0% | 0.13 | 0.13 | `bss_clear_loop+0x0` |
| `quicksort` | 1,122,333 | 1,648,354 | 1,500,617 | -9.0% | 0.68 | 0.75 | `main+0x44` |
| `store_load_forward` | 240,063 | 420,253 | 240,134 | -42.9% | 0.57 | 1.00 | `main+0x28` |
| `store_stream` | 1,376,295 | 5,273,622 | 3,211,448 | -39.1% | 0.26 | 0.43 | `bss_clear_loop+0x0` |
| `trap_return` | 120,040 | 340,185 | 540,119 | +58.8% | 0.35 | 0.22 | `trap_handler+0x0` |

Mean absolute error 16.6%, median 9.7%, over 36 programs. The unit
latencies the preset carries are measured on this RTL with
`tools/diag/rtl_probe.py`, so the kernels that exercise one unit at a time
match to within a cycle per retirement: both multiplier chains, both
dividers, the three FP kernels, the three load-to-use chains through the
L1D, `load_use`, `br_random` (the same 20,033 mispredictions on both) and
`call_return`.

What the rest comes from, by the mechanism each issue names:

- **The in-order backend retires an instruction two stages after its
  result exists (#224).** Every redirect taken from commit costs 9 cycles
  on the preset against 5 on Rocket, and a CSR read's dependent waits 5
  cycles against 3: `trap_return` (+59%), `csr_flush` (+37%),
  `csr_dependent` (+28%), `csr_serialize` (+14%) and `fence_i` (+12%).
- **A FENCE waits for older memory operations and holds younger ones
  until it retires (#228)**, where Rocket's FENCE issues at once and only
  the next memory instruction waits for the D-cache to be idle: `fence`
  (+33%), 3 cycles per iteration.
- **Rocket's D-cache replays a load that hits a pending store from WB
  (#229)**, 5 cycles each, where rvsim forwards the value in one:
  `store_load_forward` (-43%) and `indirect_calls` (-19%, the callees'
  `ld ra` two instructions after their `sd ra`).
- **Rocket's blocking D-cache stalls the core on a store miss (#231)**,
  about 41 cycles per new line, where rvsim's store buffer drains past the
  miss: `mem_write_stream` (-52%), `mem_stride` (-46%),
  `mem_dependent_miss` (-41%), `store_stream` (-39%), `mem_linear` (-32%)
  and the store loops of `matmul` (-18%) and `quicksort` (-9%).
- **Rocket's multiplier finishes early on small operands (#214)**, which
  the preset's fixed 10-cycle latency does not: `br_pattern` (+32%)
  multiplies a loop counter by a constant twice per iteration and pays
  the full latency on each, with the same 62,864 mispredictions as Rocket.
- **The latency of a miss to memory (#215)**: `chase_mem` (-18%) and
  `chase_l2` (-12%) run faster on the preset, whose memory and L2 answer
  in a fixed time where Rocket's vary between 24 and 29 cycles, while the
  pointer chases with several misses in flight run slower
  (`l2_pointer_chase` +9%, `mem_pointer_chase` +10%, `mem_random_swap`
  +8%); which part of the memory side is wrong is not traced.
- `bitmanip` (-8%), `br_nested_loops` (-9%) and `branch_taken` (-4%) are
  not traced.
