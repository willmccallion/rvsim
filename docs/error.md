# Error against gem5

How far rvsim's cycle counts are from gem5's O3 CPU on the same programs.
Measured 2026-10-03 at commit `4255224` with `make compare-gem5`.

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
| `alu_int_div` | 40,397 | 40,404 | +0.0% | 0.99 | 0.99 | 17 | 15 |
| `alu_int_mul` | 160,404 | 160,417 | +0.0% | 1.00 | 1.00 | 16 | 15 |
| `bitmanip` | 679,536 | 683,678 | +0.6% | 1.69 | 1.68 | 54 | 51 |
| `br_nested_loops` | 82,976 | 85,066 | +2.5% | 2.36 | 2.30 | 1,085 | 1,083 |
| `br_pattern` | 420,868 | 420,908 | +0.0% | 2.39 | 2.39 | 39 | 37 |
| `br_random` | 399,782 | 433,450 | +8.4% | 0.70 | 0.65 | 20,065 | 20,070 |
| `call_return` | 278,155 | 283,575 | +1.9% | 2.45 | 2.41 | 1,784 | 1,779 |
| `fp_add_chain` | 80,434 | 80,428 | -0.0% | 1.49 | 1.49 | 16 | 14 |
| `fp_fma_parallel` | 100,564 | 100,591 | +0.0% | 1.19 | 1.19 | 17 | 15 |
| `indirect_calls` | 660,606 | 750,656 | +13.6% | 0.82 | 0.72 | 30,014 | 30,007 |
| `load_use` | 183,226 | 183,297 | +0.0% | 1.32 | 1.32 | 29 | 27 |
| `matmul` | 164,377 | 190,483 | +15.9% | 1.48 | 1.27 | 779 | 724 |
| `mem_conflict` | 265,460 | 283,830 | +6.9% | 1.34 | 1.26 | 4,048 | 4,046 |
| `mem_linear` | 2,455,100 | 2,191,016 | -10.8% | 0.40 | 0.45 | 31 | 28 |
| `mem_pointer_chase` | 14,585,727 | 10,643,121 | -27.0% | 0.18 | 0.25 | 30 | 28 |
| `mem_stride` | 5,034,209 | 5,169,810 | +2.7% | 0.32 | 0.31 | 56 | 54 |
| `mem_write_stream` | 3,711,450 | 3,809,618 | +2.6% | 0.39 | 0.38 | 29 | 27 |
| `quicksort` | 1,197,282 | 1,218,114 | +1.7% | 0.94 | 0.92 | 63,966 | 63,964 |
| `store_load_forward` | 150,408 | 150,423 | +0.0% | 1.60 | 1.60 | 23 | 22 |
| `vec_axpy` | 152,016 | 161,638 | +6.3% | 0.67 | 0.63 | 55 | 55 |
| `vec_gather` | 762,503 | 606,121 | -20.5% | 0.55 | 0.69 | 54 | 52 |
| `vec_memcpy` | 633,552 | 643,153 | +1.5% | 1.01 | 0.99 | 50 | 51 |
| `vec_reduce` | 120,733 | 136,272 | +12.9% | 0.67 | 0.59 | 44 | 51 |
| `vec_strided` | 1,367,591 | 631,791 | -53.8% | 0.37 | 0.80 | 569 | 568 |

Across these 24 programs:

- **Mean absolute error:** 7.9%.
- **Median absolute error:** 2.6%.

## Store-to-load forwarding

`store_forward_latency` sets how many cycles a load forwarded from the
store buffer takes to write back. The comparison runs at `1`: gem5's LSQ
schedules a forwarded load's writeback event in the cycle it executes,
but the event runs after the CPU's tick has advanced the writeback queue,
so the load writes back in the next cycle, one cycle after it executes.
On this machine that is also the L1D hit latency, rvsim's default.

| `store_forward_latency` | `store_load_forward` cycles | Error | `indirect_calls` cycles | Error |
|---|---:|---:|---:|---:|
| `1` (gem5, and the L1D hit latency) | 150,423 | +0.0% | 750,656 | +13.6% |
| `0` (written back in the cycle it matches) | 90,453 | -39.9% | 690,672 | +4.6% |

No other program moves by more than a few cycles between the two
settings. The forwarding kernel matches gem5 at `1`, so the
`indirect_calls` gap is not forwarding latency: at `0` rvsim is still
slower than gem5 there, and the setting is wrong for the kernel that
measures it directly.

## Caveats

- The gem5 numbers are the stored `tools/gem5_compare/results/gem5.json`,
  recorded 2026-09-28. No gem5 binary is built on this machine, so they
  were not rerun. Both simulators retire the same instruction counts to
  within 0.4%.
- `mem_dependent_miss` and `mem_random_swap` have no gem5 result and are
  left out.
- gem5 runs in syscall-emulation mode; rvsim runs bare-metal. Both use a
  fixed-latency memory.

## Known causes

- **Compute-bound kernels match.** The ALU and FP chains, `load_use` and
  `store_load_forward` are within 0.1%: unit latencies, the load pipeline
  and forwarding agree with gem5's O3 defaults cycle for cycle.
- **Branch-heavy kernels run 2-14% slow.** `br_random`, `indirect_calls`
  and `call_return` mispredict as often as gem5 but recover more slowly.
  Not yet root-caused.
- **Streaming and pointer-chasing kernels run 11-27% fast; strided,
  write-stream and conflict kernels 3-7% slow.** rvsim has no `L2XBar`
  between the L1s and the L2, so a miss that goes to memory is cheaper,
  while its L1D miss handling under conflicts is dearer. `matmul`, which
  mixes both, is 16% slow and not yet root-caused.
- **Vector kernels differ both ways.** rvsim models lanes and moves a
  datapath width per cycle where gem5 splits an instruction into one
  micro-op per register of its group; strided and indexed accesses are
  the furthest apart.
