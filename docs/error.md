# Error against gem5

How far rvsim's cycle counts are from gem5's O3 CPU on the same programs.
Measured 2026-10-03 at commit `4255224` with `make compare-gem5`, with
gem5 rerun from the checkout under `tests/builds/gem5-work`.

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
| `bitmanip` | 679,536 | 683,601 | +0.6% | 1.69 | 1.68 | 54 | 51 |
| `br_nested_loops` | 82,976 | 83,999 | +1.2% | 2.36 | 2.33 | 1,085 | 1,083 |
| `br_pattern` | 420,868 | 420,882 | +0.0% | 2.39 | 2.39 | 39 | 37 |
| `br_random` | 399,782 | 383,879 | -4.0% | 0.70 | 0.73 | 20,065 | 20,070 |
| `call_return` | 278,155 | 275,097 | -1.1% | 2.45 | 2.48 | 1,784 | 1,779 |
| `fp_add_chain` | 80,434 | 80,441 | +0.0% | 1.49 | 1.49 | 16 | 14 |
| `fp_fma_parallel` | 100,564 | 100,577 | +0.0% | 1.19 | 1.19 | 17 | 15 |
| `indirect_calls` | 660,606 | 660,666 | +0.0% | 0.82 | 0.82 | 30,014 | 30,007 |
| `load_use` | 183,226 | 183,295 | +0.0% | 1.32 | 1.32 | 29 | 27 |
| `matmul` | 164,377 | 186,051 | +13.2% | 1.48 | 1.31 | 779 | 727 |
| `mem_conflict` | 265,460 | 243,792 | -8.2% | 1.34 | 1.46 | 4,048 | 4,046 |
| `mem_dependent_miss` | 3,156,080 | 3,223,368 | +2.1% | 0.36 | 0.35 | 29 | 27 |
| `mem_linear` | 2,455,100 | 2,190,991 | -10.8% | 0.40 | 0.45 | 31 | 28 |
| `mem_pointer_chase` | 14,585,727 | 10,643,101 | -27.0% | 0.18 | 0.25 | 30 | 28 |
| `mem_random_swap` | 7,948,188 | 5,136,384 | -35.4% | 0.15 | 0.23 | 27 | 26 |
| `mem_stride` | 5,034,209 | 5,169,542 | +2.7% | 0.32 | 0.31 | 56 | 54 |
| `mem_write_stream` | 3,711,450 | 3,809,634 | +2.6% | 0.39 | 0.38 | 29 | 27 |
| `quicksort` | 1,197,282 | 1,198,825 | +0.1% | 0.94 | 0.94 | 63,966 | 63,961 |
| `store_load_forward` | 150,408 | 150,421 | +0.0% | 1.60 | 1.60 | 23 | 22 |
| `vec_axpy` | 152,016 | 161,648 | +6.3% | 0.67 | 0.63 | 55 | 55 |
| `vec_gather` | 762,503 | 606,110 | -20.5% | 0.55 | 0.69 | 54 | 52 |
| `vec_memcpy` | 633,552 | 643,154 | +1.5% | 1.01 | 0.99 | 50 | 51 |
| `vec_reduce` | 120,733 | 136,221 | +12.8% | 0.67 | 0.59 | 44 | 51 |
| `vec_strided` | 1,367,591 | 631,496 | -53.8% | 0.37 | 0.80 | 569 | 568 |

Across these 26 programs:

- **Mean absolute error:** 7.9%.
- **Median absolute error:** 1.8%.

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

## Caveats

- gem5 is v24.0.0.1, built under `tests/builds/gem5-work/gem5`; its
  numbers reproduce the ones recorded 2026-09-28 exactly. Both simulators
  retire the same instruction counts to within 0.4%.
- gem5 runs in syscall-emulation mode; rvsim runs bare-metal. Both use a
  fixed-latency memory.

## Known causes

- **Compute-bound and branch-bound kernels match.** The ALU and FP
  chains, `load_use`, `store_load_forward`, `indirect_calls`,
  `br_pattern` and `quicksort` are within 0.2%; `br_random`,
  `call_return` and `br_nested_loops` within 4%.
- **Streaming and pointer-chasing kernels run 11-35% fast; strided,
  write-stream and dependent-miss kernels 2-3% slow.** rvsim has no
  `L2XBar` between the L1s and the L2, so a miss that goes to memory is
  cheaper, while its L1D miss handling under conflicts is dearer
  (`mem_conflict` 8% fast). `matmul`, which mixes both, is 13% slow and
  not yet root-caused.
- **Vector kernels differ both ways.** rvsim models lanes and moves a
  datapath width per cycle where gem5 splits an instruction into one
  micro-op per register of its group; strided and indexed accesses are
  the furthest apart.
