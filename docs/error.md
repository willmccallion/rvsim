# Error against gem5

How far rvsim's cycle counts are from gem5's O3 CPU on the same programs.
gem5 is the reference rvsim can be compared with cycle by cycle, not the
target: where gem5 times something the way a simulator does rather than
the way a core does, rvsim follows the core and the difference stays in
these numbers ([decision 12](architecture/decisions/0012-the-model-follows-real-cores.md)).
Hardware is the other reference; see [Linux Benchmarks](examples/linux-benchmarks.md).

Measured 2026-11-23 with `make compare-gem5`, gem5 rerun from the
checkout under `tests/builds/gem5-work`.

## Method

Both simulators run the programs in `tools/gem5_compare/programs`
(binaries in `tests/builds/compare-programs`), small C kernels built at
`-O2` for `rv64gcv_zba_zbb_zbc_zbs_zbkb_zbkx_zfh` that each isolate one
behaviour: branch patterns, calls and indirect calls, memory access
patterns, ALU and FP latency chains, store-to-load forwarding, and vector
unit-stride, strided, indexed and reduction code. The machine is the
P550-like base in `tools/gem5_compare/variants.py`: 3-wide O3, 72-entry
ROB, tournament predictor, 32 KiB L1s, 256 KiB L2, 1.4 GHz, fixed-latency
memory at 12.8 GiB/s. `tools/gem5_compare/README.md` lists what rvsim
cannot express of gem5's machine.

Both run the same ELF, gem5 in syscall-emulation mode and rvsim
bare-metal: the programs' own start-up (`tools/gem5_compare/programs/start.s`)
takes the stack and the FPU and vector unit as gem5's loader leaves them,
which `run_rvsim.py` sets up the same way, and ends with the m5 exit
pseudo-instruction both simulators take.

Error is `(rvsim cycles - gem5 cycles) / gem5 cycles`, over the whole
program. A positive error means rvsim is slower than gem5.

## Results

| Program | gem5 cycles | rvsim cycles | Error | gem5 IPC | rvsim IPC | gem5 mispredicts | rvsim mispredicts |
|---|---:|---:|---:|---:|---:|---:|---:|
| `alu_int_div` | 40,381 | 40,393 | +0.0% | 0.99 | 0.99 | 17 | 15 |
| `alu_int_mul` | 160,390 | 160,400 | +0.0% | 1.00 | 1.00 | 16 | 14 |
| `bitmanip` | 671,327 | 684,619 | +2.0% | 1.71 | 1.68 | 54 | 51 |
| `br_nested_loops` | 82,962 | 83,986 | +1.2% | 2.36 | 2.33 | 1,085 | 1,083 |
| `br_pattern` | 420,859 | 420,867 | +0.0% | 2.39 | 2.39 | 39 | 37 |
| `br_random` | 406,725 | 383,913 | -5.6% | 0.69 | 0.73 | 20,065 | 20,068 |
| `call_return` | 289,665 | 288,893 | -0.3% | 2.35 | 2.36 | 1,784 | 1,779 |
| `fp_add_chain` | 80,500 | 80,476 | -0.0% | 1.49 | 1.49 | 16 | 20 |
| `fp_fma_parallel` | 100,533 | 100,549 | +0.0% | 1.19 | 1.19 | 17 | 15 |
| `indirect_calls` | 660,619 | 615,690 | -6.8% | 0.82 | 0.88 | 30,014 | 30,007 |
| `load_use` | 183,236 | 183,335 | +0.1% | 1.32 | 1.32 | 29 | 27 |
| `matmul` | 165,013 | 173,272 | +5.0% | 1.47 | 1.40 | 779 | 726 |
| `mem_conflict` | 265,697 | 244,319 | -8.0% | 1.34 | 1.46 | 4,048 | 4,046 |
| `mem_dependent_miss` | 3,140,620 | 3,223,208 | +2.6% | 0.36 | 0.35 | 29 | 27 |
| `mem_linear` | 2,455,316 | 2,206,641 | -10.1% | 0.40 | 0.45 | 31 | 28 |
| `mem_pointer_chase` | 13,962,525 | 10,669,785 | -23.6% | 0.19 | 0.25 | 30 | 28 |
| `mem_random_swap` | 7,537,725 | 3,027,732 | -59.8% | 0.16 | 0.39 | 27 | 26 |
| `mem_stride` | 5,034,283 | 5,204,378 | +3.4% | 0.32 | 0.31 | 56 | 54 |
| `mem_write_stream` | 3,711,413 | 3,834,152 | +3.3% | 0.39 | 0.38 | 29 | 27 |
| `quicksort` | 1,188,168 | 1,191,175 | +0.3% | 0.94 | 0.94 | 63,971 | 63,956 |
| `store_load_forward` | 150,402 | 120,417 | -19.9% | 1.60 | 1.99 | 23 | 22 |
| `vec_axpy` | 152,103 | 160,124 | +5.3% | 0.67 | 0.64 | 55 | 55 |
| `vec_gather` | 763,925 | 609,411 | -20.2% | 0.55 | 0.69 | 54 | 52 |
| `vec_memcpy` | 620,871 | 637,382 | +2.7% | 1.03 | 1.00 | 50 | 51 |
| `vec_reduce` | 120,724 | 136,707 | +13.2% | 0.67 | 0.59 | 44 | 51 |
| `vec_strided` | 1,371,413 | 633,716 | -53.8% | 0.37 | 0.79 | 569 | 563 |

Across these 26 programs:

- **Mean absolute error:** 9.5%.
- **Median absolute error:** 3.3%.

## Misprediction recovery

gem5's `O3PipeView` trace and rvsim's `RUST_LOG=rvsim=trace` output were
lined up on `br_random`, cycle by cycle, around each mispredicted branch.
Relative to the cycle the branch's result completes, both simulators now
fetch the correct path at +3, decode it at +4, rename it at +6 and issue
it at +8: fetch spends the squash cycle squashing; commit squashes the ROB
at `squash_width` (8) entries per cycle; dispatch holds while it sees
commit squashing and rename while it sees dispatch held, each a cycle
late. Before this was modelled rvsim renamed the correct path at +9 and
issued it at +11, three cycles behind gem5 on every misprediction, which
is where the 2-14% on the branch-heavy kernels came from.

What is left on those kernels: rvsim retires a completed instruction one
cycle after it completes, gem5 two (`iewToCommitDelay` and the commit
cycle), so rvsim frees ROB entries a cycle earlier; and rvsim's wrong path
puts a few more instructions into the ROB before the squash, so its
squash takes a cycle longer at times. Together they leave `br_random`
1.1 cycles per misprediction ahead of gem5.

## Stores issue address and data separately

rvsim issues a plain store's address as soon as its base register is
ready and its data when the value is, as real out-of-order cores split a
store into store-address and store-data operations. gem5's O3 issues a
store only when both registers are ready, so a younger load that waits
for older stores' addresses also waits for their data. rvsim does not
model that, and the kernels whose stores take their data from a long
chain run faster than in gem5 for it (measured 2026-10-04, on the
previous build of the programs):

| Program | Error with whole stores | Error with split stores |
|---|---:|---:|
| `store_load_forward` | +0.0% | -19.9% |
| `mem_random_swap` | -35.2% | -61.9% |
| `indirect_calls` | +0.0% | -7.9% |

The other programs move by under a point. These differences are gem5's,
not rvsim's, and are left in the comparison on purpose.

## Store-to-load forwarding

`store_forward_latency` sets how many cycles a load forwarded from the
store buffer takes to write back. The comparison runs at `1`: gem5's LSQ
schedules a forwarded load's writeback event in the cycle it executes,
but the event runs after the CPU's tick has advanced the writeback queue,
so the load writes back in the next cycle, one cycle after it executes.
On this machine that is also the L1D hit latency, rvsim's default. The
table was measured 2026-10-04, on the previous build of the programs.

| `store_forward_latency` | `store_load_forward` cycles | Error |
|---|---:|---:|
| `1` (gem5, and the L1D hit latency) | 150,421 | +0.0% |
| `0` (written back in the cycle it matches) | 90,453 | -39.9% |

Both rows were measured with stores issuing whole, as gem5 issues them;
with split stores the forwarding kernel runs 20% faster than gem5 at `1`,
as above.

## Caveats

- gem5 is v24.0.0.1, built under `tests/builds/gem5-work/gem5`; run on
  the previous build of the programs it reproduces the numbers recorded
  2026-09-28 exactly. rvsim retires one instruction more than gem5 in
  every program: it counts the m5 exit, which gem5 does not.
- Both use a fixed-latency memory.
- gem5's cycle counts move with where the linker puts the code and data:
  rebuilding the programs with today's start-up, a few instructions
  shorter than the one before, took 4.3% off `mem_pointer_chase` in gem5
  and under 0.1% in rvsim.

## Known causes

- **Compute-bound and branch-bound kernels match.** The ALU and FP
  chains, `load_use`, `br_pattern`, `call_return` and `quicksort` are
  within 0.3%; `bitmanip` and `br_nested_loops` within 2%, and
  `br_random` 5.6% fast.
- **Split stores make three kernels faster than gem5.**
  `store_load_forward` (-19.9%), `indirect_calls` (-6.8%) and most of
  `mem_random_swap` (-59.8%), as described above; these are gem5's
  differences from hardware.
- **Streaming and pointer-chasing kernels run 10-24% fast; strided,
  write-stream and dependent-miss kernels 3% slow.** rvsim has no
  `L2XBar` between the L1s and the L2, so a miss that goes to memory is
  cheaper, while its L1D miss handling under conflicts differs
  (`mem_conflict` 8% fast). `matmul`, which mixes both, is 5% slow and
  not yet root-caused.
- **Vector kernels differ both ways.** rvsim models lanes and moves a
  datapath width per cycle where gem5 splits an instruction into one
  micro-op per register of its group; strided and indexed accesses are
  the furthest apart.
