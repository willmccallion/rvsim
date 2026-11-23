# Error against BOOM

How far rvsim's retire timing is from BOOM's, instruction by instruction,
on the same programs. BOOM is Berkeley's out-of-order core; its V4 RTL is
built as a Verilator simulator from Chipyard, so every cycle difference
on a matched configuration is an rvsim bug, a missing model feature, or a
documented decision
([decision 12](architecture/decisions/0012-the-model-follows-real-cores.md)).
The in-order counterpart is [Error against Rocket](rtl-rocket.md), which
also describes the method, the programs and the memory model; the gem5
comparison is [Error against gem5](error.md).

## Machine

BOOM as `MediumBoomV4Config` builds it (`boom.v4.common.WithNMediumBooms`
in `v4/common/config-mixins.scala`, `chipyard.config.AbstractConfig`),
with its commit log on (`tools/rtl/RvsimConfigs.scala`):

- 2-wide decode and commit, 4-wide fetch, 64-entry ROB, 12 branch
  checkpoints (`maxBrCount`), a 16-entry fetch buffer and a 32-entry fetch
  target queue.
- Issue queues: memory 12 entries issuing 2, unique (CSR and system) 12
  issuing 1, ALU 20 issuing 2, FP 12 issuing 1; 80 integer and 64 FP
  physical registers; 16-entry load and store queues.
- `FPUParams(sfmaLatency = 4, dfmaLatency = 4, divSqrt = true)`; the
  default `MulDivParams` (`divEarlyOut`); Zba, Zbb and Zbs.
- 16 KiB 4-way L1I (`ICacheParams(nSets = 64, nWays = 4, fetchBytes = 8)`)
  and 16 KiB 4-way L1D (`DCacheParams(nSets = 64, nWays = 4, nMSHRs = 2,
  nTLBWays = 8)`), 64-byte lines.
- TAGE-L (`WithTAGELBPD`): six tagged tables of (sets, history, tag bits)
  (128, 2, 7), (128, 4, 7), (256, 8, 8), (256, 16, 8), (128, 32, 9),
  (128, 64, 9) with a useful-bit reset every 2048 updates
  (`BoomTageParams`), over a 2048-set bimodal table (`BoomBIMParams`), a
  128-set 2-way BTB (`BoomBTBParams`), a 16-way fully associative micro-BTB
  (`BoomFAMicroBTBParams`), a 4-way loop predictor and a 32-entry return
  address stack (`numRasEntries`); 64 bits of global history.
- 512-entry L2 TLB (`nL2TLBEntries`); 512 KiB 8-way inclusive L2
  (`WithInclusiveCache` defaults).

Chipyard 1.14.0's 3-wide `LargeBoomV4Config` hangs on the boot ROM's first
CSR write, with or without BOOM's later fix for its banked issue unit, and
Chipyard's own CI builds only the Medium configuration, so the comparison
uses Medium. The rvsim preset is `presets.boom()`; each parameter there
cites the line it comes from or the probe it was measured with.

## Results

Chipyard 1.14.0's `MediumBoomV4Config` against `presets.boom()`, after
#211, #212, #213 and #87. The columns are as on the Rocket page.

| program | instructions | BOOM cycles | rvsim cycles | error | BOOM IPC | rvsim IPC | where the difference accumulates |
|---|---:|---:|---:|---:|---:|---:|---|
| `alu_int_div` | 40,042 | 72,166 | 72,085 | -0.1% | 0.55 | 0.56 | `main+0x24` |
| `alu_int_mul` | 160,043 | 320,169 | 320,087 | -0.0% | 0.50 | 0.50 | `main+0x28` |
| `bitmanip` | 1,146,949 | 959,831 | 892,154 | -7.1% | 1.19 | 1.29 | `main+0x44` |
| `br_nested_loops` | 196,037 | 104,561 | 118,418 | +13.3% | 1.87 | 1.66 | `main+0x2e` |
| `br_pattern` | 1,005,760 | 630,254 | 549,596 | -12.8% | 1.60 | 1.83 | `main+0x54` |
| `br_random` | 280,053 | 609,545 | 592,531 | -2.8% | 0.46 | 0.47 | `main+0x4c` |
| `branch_taken` | 690,037 | 390,289 | 690,132 | +76.8% | 1.77 | 1.00 | `main+0x16` |
| `call_return` | 682,020 | 483,587 | 397,447 | -17.8% | 1.41 | 1.72 | `fib+0x2` |
| `chase_l1` | 83,570 | 324,821 | 325,883 | +0.3% | 0.26 | 0.26 | `main+0x76` |
| `chase_l2` | 265,331 | 1,287,916 | 1,271,267 | -1.3% | 0.21 | 0.21 | `main+0x72` |
| `chase_mem` | 3,624,529 | 8,175,767 | 8,571,950 | +4.8% | 0.44 | 0.42 | `main+0x72` |
| `csr_dependent` | 100,036 | 440,181 | 320,080 | -27.3% | 0.23 | 0.31 | `main+0x10` |
| `csr_flush` | 60,037 | 440,183 | 320,083 | -27.3% | 0.14 | 0.19 | `main+0xe` |
| `csr_serialize` | 100,037 | 740,178 | 560,083 | -24.3% | 0.14 | 0.18 | `main+0x12` |
| `div_chain` | 32,040 | 448,202 | 448,070 | -0.0% | 0.07 | 0.07 | `main+0x24` |
| `fence` | 160,232 | 580,354 | 480,293 | -17.2% | 0.28 | 0.33 | `main+0x2c` |
| `fence_i` | 60,037 | 720,224 | 780,145 | +8.3% | 0.08 | 0.08 | `main+0x12` |
| `fp_add_chain` | 120,040 | 200,194 | 200,086 | -0.1% | 0.60 | 0.60 | `main+0x24` |
| `fp_div_chain` | 24,040 | 480,136 | 480,075 | -0.0% | 0.05 | 0.05 | `main+0x24` |
| `fp_fma_parallel` | 120,050 | 100,234 | 100,131 | -0.1% | 1.20 | 1.20 | `main+0x40` |
| `indirect_calls` | 540,049 | 963,991 | 918,912 | -4.7% | 0.56 | 0.59 | `main+0x2e` |
| `l2_pointer_chase` | 782,637 | 2,452,346 | 3,040,548 | +24.0% | 0.32 | 0.26 | `main+0x64` |
| `load_use` | 242,090 | 361,728 | 361,894 | +0.0% | 0.67 | 0.67 | `main+0x38` |
| `matmul` | 722,021 | 627,305 | 632,061 | +0.8% | 1.15 | 1.14 | `main+0x9e` |
| `mem_conflict` | 356,616 | 2,105,093 | 824,169 | -60.8% | 0.17 | 0.43 | `main+0x22` |
| `mem_dependent_miss` | 1,128,619 | 1,888,130 | 1,588,980 | -15.8% | 0.60 | 0.71 | `bss_clear_loop+0x0` |
| `mem_linear` | 983,083 | 1,001,003 | 692,922 | -30.8% | 0.98 | 1.42 | `main+0x16` |
| `mem_pointer_chase` | 2,650,413 | 7,510,357 | 9,598,398 | +27.8% | 0.35 | 0.28 | `main+0x68` |
| `mem_random_swap` | 1,179,693 | 3,131,599 | 4,067,472 | +29.9% | 0.38 | 0.29 | `main+0x62` |
| `mem_stride` | 1,613,888 | 2,800,964 | 2,245,735 | -19.8% | 0.58 | 0.72 | `bss_clear_loop+0x0` |
| `mem_write_stream` | 1,441,835 | 1,766,929 | 1,544,288 | -12.6% | 0.82 | 0.93 | `main+0x30` |
| `mul_chain` | 50,042 | 280,167 | 280,077 | -0.0% | 0.18 | 0.18 | `main+0x46` |
| `quicksort` | 1,122,333 | 1,959,262 | 1,549,616 | -20.9% | 0.57 | 0.72 | `sort.constprop.0+0x50` |
| `store_load_forward` | 240,063 | 540,216 | 210,129 | -61.1% | 0.44 | 1.14 | `main+0x28` |
| `store_stream` | 1,376,295 | 3,176,509 | 2,723,990 | -14.2% | 0.43 | 0.51 | `bss_clear_loop+0x0` |
| `trap_return` | 140,040 | 1,440,222 | 1,180,100 | -18.1% | 0.10 | 0.12 | `trap_handler+0x6` |

Mean absolute error 16.2%, median 13.0%, over 36 programs. The kernels
that exercise one unit at a time match: both multiplier chains, both
dividers, the three FP kernels, the L1 and L2 load-to-use chains,
`load_use` and `matmul`.

What the rest comes from:

- **The out-of-order backend has no dispatch, register-read or separate
  writeback stage (#226).** A redirect from commit reaches the next
  retirement in 12 to 13 cycles on the preset against 13 to 14 on BOOM:
  `csr_serialize`, `csr_flush` and `csr_dependent` (-24% to -27%),
  `trap_return` (-18%) and `fence` (-17%). The preset's `trap_latency=1`,
  `fetch_decode_latency=3` and `rename_issue_latency=4` are calibrations
  that stand in for those stages on the trap and mispredict paths; with
  them a misprediction costs the preset 11 cycles and a trap 14 to its
  handler, as each costs BOOM.
- **BOOM issues a load past unresolved older stores and flushes on an
  ordering failure (#230)**, where the preset's `Blind` policy holds the
  load: `mem_conflict` (-61%), `store_load_forward` (-61%) and
  `quicksort` (-21%).
- **Fetch keeps one group in flight (#232)**, a group every I-cache
  latency plus one cycles, where BOOM's front end fetches a packet every
  cycle: `branch_taken` (+77%, an inner loop of four instructions that
  BOOM runs at 1.77 IPC and the preset at 1.00, with six mispredictions
  on either side) and `br_nested_loops` (+13%).
- **The latency of a miss to memory (#215)**: `l2_pointer_chase` (+24%),
  `mem_pointer_chase` (+28%), `mem_random_swap` (+30%) and `chase_mem`
  (+5%) run slower on the preset; which part of the memory side is wrong
  is not traced.
- **Streaming stores and loads** run faster on the preset: `mem_linear`
  (-31%), `mem_stride` (-20%), `mem_dependent_miss` (-16%), `store_stream`
  (-14%) and `mem_write_stream` (-13%), with the difference on their store
  and load loops. The same kernels on Rocket are its blocking D-cache (#231);
  what BOOM's two-MSHR L1D does with them is not traced.
- `fence_i` (+8%) has run slower since the misprediction calibration
  (#87): the refill after its I-cache flush pays the preset's added
  front-end and rename-to-issue cycles, and where BOOM's refill differs
  is not traced.
- `call_return` (-18%), `br_pattern` (-13%), `bitmanip` (-7%),
  `indirect_calls` (-5%) and `br_random` (-3%) run faster on the preset,
  whose TAGE and return stack are configured as BOOM's but not compared
  prediction by prediction; not traced.
