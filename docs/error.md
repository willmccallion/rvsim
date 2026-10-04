# Error against gem5

How far rvsim's cycle counts are from gem5's O3 CPU on the same programs.
gem5 is the reference rvsim can be compared with cycle by cycle, not the
target: where gem5 times something the way a simulator does rather than
the way a core does, rvsim follows the core and the difference stays in
these numbers ([decision 12](architecture/decisions/0012-the-model-follows-real-cores.md)).
Hardware is the other reference; see [Linux Benchmarks](examples/linux-benchmarks.md).

Measured 2026-10-04, after stores were split into address and data
halves, with `make compare-gem5` and gem5 rerun from the checkout under
`tests/builds/gem5-work`.

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

Error is `(rvsim cycles - gem5 cycles) / gem5 cycles`, over the whole
program. A positive error means rvsim is slower than gem5.

## Results

| Program | gem5 cycles | rvsim cycles | Error | gem5 IPC | rvsim IPC | gem5 mispredicts | rvsim mispredicts |
|---|---:|---:|---:|---:|---:|---:|---:|
| `alu_int_div` | 40,397 | 40,405 | +0.0% | 0.99 | 0.99 | 17 | 15 |
| `alu_int_mul` | 160,404 | 160,412 | +0.0% | 1.00 | 1.00 | 16 | 14 |
| `bitmanip` | 679,536 | 684,619 | +0.7% | 1.69 | 1.68 | 54 | 51 |
| `br_nested_loops` | 82,976 | 83,999 | +1.2% | 2.36 | 2.33 | 1,085 | 1,083 |
| `br_pattern` | 420,868 | 420,882 | +0.0% | 2.39 | 2.39 | 39 | 37 |
| `br_random` | 399,782 | 383,879 | -4.0% | 0.70 | 0.73 | 20,065 | 20,070 |
| `call_return` | 278,155 | 278,042 | -0.0% | 2.45 | 2.45 | 1,784 | 1,779 |
| `fp_add_chain` | 80,434 | 80,440 | +0.0% | 1.49 | 1.49 | 16 | 14 |
| `fp_fma_parallel` | 100,564 | 100,575 | +0.0% | 1.19 | 1.19 | 17 | 15 |
| `indirect_calls` | 660,606 | 608,192 | -7.9% | 0.82 | 0.89 | 30,014 | 30,007 |
| `load_use` | 183,226 | 183,311 | +0.0% | 1.32 | 1.32 | 29 | 27 |
| `matmul` | 164,377 | 187,811 | +14.3% | 1.48 | 1.29 | 779 | 727 |
| `mem_conflict` | 265,460 | 244,303 | -8.0% | 1.34 | 1.46 | 4,048 | 4,046 |
| `mem_dependent_miss` | 3,156,080 | 3,239,750 | +2.7% | 0.36 | 0.35 | 29 | 27 |
| `mem_linear` | 2,455,100 | 2,205,327 | -10.2% | 0.40 | 0.45 | 31 | 28 |
| `mem_pointer_chase` | 14,585,727 | 10,664,847 | -26.9% | 0.18 | 0.25 | 30 | 28 |
| `mem_random_swap` | 7,948,188 | 3,027,287 | -61.9% | 0.15 | 0.39 | 27 | 26 |
| `mem_stride` | 5,034,209 | 5,202,828 | +3.3% | 0.32 | 0.31 | 56 | 54 |
| `mem_write_stream` | 3,711,450 | 3,834,208 | +3.3% | 0.39 | 0.38 | 29 | 27 |
| `quicksort` | 1,197,282 | 1,200,092 | +0.2% | 0.94 | 0.94 | 63,966 | 63,961 |
| `store_load_forward` | 150,408 | 120,429 | -19.9% | 1.60 | 1.99 | 23 | 22 |
| `vec_axpy` | 152,016 | 164,203 | +8.0% | 0.67 | 0.62 | 55 | 55 |
| `vec_gather` | 762,503 | 608,586 | -20.2% | 0.55 | 0.69 | 54 | 52 |
| `vec_memcpy` | 633,552 | 645,203 | +1.8% | 1.01 | 0.99 | 50 | 51 |
| `vec_reduce` | 120,733 | 136,735 | +13.3% | 0.67 | 0.59 | 44 | 51 |
| `vec_strided` | 1,367,591 | 633,560 | -53.7% | 0.37 | 0.79 | 569 | 568 |

Across these 26 programs:

- **Mean absolute error:** 10.1%.
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
0.8 cycles per misprediction ahead of gem5.

## Stores issue address and data separately

rvsim issues a plain store's address as soon as its base register is
ready and its data when the value is, as real out-of-order cores split a
store into store-address and store-data operations. gem5's O3 issues a
store only when both registers are ready, so a younger load that waits
for older stores' addresses also waits for their data. rvsim does not
model that, and the kernels whose stores take their data from a long
chain run faster than in gem5 for it:

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
On this machine that is also the L1D hit latency, rvsim's default.

| `store_forward_latency` | `store_load_forward` cycles | Error |
|---|---:|---:|
| `1` (gem5, and the L1D hit latency) | 150,421 | +0.0% |
| `0` (written back in the cycle it matches) | 90,453 | -39.9% |

Both rows were measured with stores issuing whole, as gem5 issues them;
with split stores the forwarding kernel runs 20% faster than gem5 at `1`,
as above.

## Caveats

- gem5 is v24.0.0.1, built under `tests/builds/gem5-work/gem5`; its
  numbers reproduce the ones recorded 2026-09-28 exactly. Both simulators
  retire the same instruction counts to within 0.4%.
- gem5 runs in syscall-emulation mode; rvsim runs bare-metal. Both use a
  fixed-latency memory.

## Known causes

- **Compute-bound and branch-bound kernels match.** The ALU and FP
  chains, `load_use`, `br_pattern`, `call_return` and `quicksort` are
  within 0.2%; `bitmanip`, `br_nested_loops` and `br_random` within 4%.
- **Split stores make three kernels faster than gem5.**
  `store_load_forward` (-19.9%), `indirect_calls` (-7.9%) and most of
  `mem_random_swap` (-61.9%), as described above; these are gem5's
  differences from hardware.
- **Streaming and pointer-chasing kernels run 10-27% fast; strided,
  write-stream and dependent-miss kernels 3% slow.** rvsim has no
  `L2XBar` between the L1s and the L2, so a miss that goes to memory is
  cheaper, while its L1D miss handling under conflicts differs
  (`mem_conflict` 8% fast). `matmul`, which mixes both, is 14% slow and
  not yet root-caused.
- **Vector kernels differ both ways.** rvsim models lanes and moves a
  datapath width per cycle where gem5 splits an instruction into one
  micro-op per register of its group; strided and indexed accesses are
  the furthest apart.
