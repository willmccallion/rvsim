# ISA

rvsim implements **RV64GC with the vector extension** and the
supervisor-level privileged architecture: enough to boot Linux through
OpenSBI and to run the vector, bit-manipulation and cryptography code
modern compilers emit. Every instruction's behaviour is defined once, in
the shared execute layer, and both pipelines run that definition
([decision 10](decisions/0010-semantics-are-separate-from-timing.md)).

`misa` reads `RV64IMAFDCBSUV`, B standing for Zba, Zbb and Zbs. Every
extension below is always on except
Svadu, which `Config(svadu=True)` enables; the vector unit's VLEN is set by
`Config(vlen=...)`.

## Unprivileged extensions

| Extension | What it adds |
|-----------|--------------|
| **I** | The 64-bit base integer set, with the `W` forms that operate on 32 bits and sign-extend |
| **M** | Multiply, divide and remainder, with the specification's results for division by zero and overflow |
| **A** | `LR`/`SC` and the nine AMOs, word and doubleword, with `aq` and `rl` ordering |
| **F**, **D** | Single- and double-precision IEEE 754 arithmetic, fused multiply-add, conversions, comparisons, sign injection and classification; all five rounding modes and the accrued exception flags |
| **C** | 16-bit compressed encodings, expanded to their 32-bit forms in Fetch2; mixed 16/32-bit streams and instructions that straddle a line or a page |
| **Zicsr**, **Zifencei** | CSR access instructions and `FENCE.I` |
| **Zicntr** | `cycle`, `time` and `instret`, gated for S and U by `mcounteren` and `scounteren` |
| **Zba**, **Zbb**, **Zbc**, **Zbs** | Address generation (`sh*add`, `add.uw`, `slli.uw`), basic bit manipulation (`clz`, `ctz`, `cpop`, `min`/`max`, rotates, `rev8`, `orc.b`, sign and zero extension), carry-less multiply, and single-bit operations |
| **Zbkb**, **Zbkx** | Bit manipulation for cryptography (`brev8`, `pack`, `packh`, `packw`) and crossbar permutations (`xperm4`, `xperm8`) |
| **Zfh** | Half-precision floating point, NaN-boxed in the `f` registers |
| **Zicbom**, **Zicboz** | `cbo.clean`, `cbo.flush` and `cbo.inval` on a 64-byte block, and `cbo.zero`; see [cache-block operations](memory.md#cache-block-operations) |
| **V** | RVV 1.0, below |

### Floating point

Single-precision (and half-precision) values in the 64-bit `f` registers
are NaN-boxed: a value whose upper bits are not all ones reads as the
canonical NaN. `mstatus.FS` tracks the floating-point state: an FP
instruction with FS Off raises an illegal-instruction exception, and one
that writes an `f` register or `fflags` sets FS to Dirty. The same holds
for `mstatus.VS` and the vector state.

### Memory accesses

Misaligned loads and stores are performed in hardware by default, split
across lines and pages where needed; with
`Config(misaligned_access_trap=True)` they raise address-misaligned
exceptions instead. Misaligned atomics always raise one, as the
specification requires.

`LR` reserves the 64-byte line holding its address and `SC` succeeds only
while the reservation holds; a store by another hart, or a device's DMA,
anywhere in the line breaks it, and every `SC` clears it. AMOs and `SC`
execute as the oldest instruction once every older store has been
written, and take effect in the L1D.

## Vector extension

RVV 1.0 with ELEN 64 and VLEN a power of two from 128 to 2048 bits
(default 128): loads and stores (unit-stride, strided, indexed ordered and
unordered, segment, fault-only-first, mask and whole-register), integer,
fixed-point (with `vxrm` rounding and `vxsat`), floating point including
half precision (**Zvfh**), widening and narrowing forms, reductions, mask
operations and permutations (slides, gathers, compress, `vmv`), all at
every LMUL including the fractional ones, with tail- and mask-agnostic
policies.

| Sub-extension | What it adds |
|---------------|--------------|
| **Zvbb** | Vector bit manipulation: `vandn`, `vbrev`, `vbrev8`, `vrev8`, `vclz`, `vctz`, `vcpop`, `vrol`, `vror`, `vwsll` |
| **Zvbc** | Vector carry-less multiply: `vclmul`, `vclmulh` |
| **Zvkn** | NIST suite: AES (Zvkned), SHA-256 and SHA-512 (Zvknha, Zvknhb), and Zvkb |
| **Zvks** | ShangMi suite: SM4 (Zvksed), SM3 (Zvksh), and Zvkb |
| **Zvkg** | GHASH for AES-GCM |

The crypto instructions operate on element groups as the vector crypto
specification defines, and several re-use the `vs1` field as a sub-opcode
or an immediate rather than a register.

The vector CSRs are `vstart`, `vxsat`, `vxrm`, `vcsr`, `vl`, `vtype` and
`vlenb`. `vsetvl` with an unsupported `vtype` sets `vill`.

How long vector instructions take is described in
[Pipeline](pipeline.md): vector arithmetic occupies its unit for a time set
by `vl` and the lane count, and vector memory accesses move up to
`vector_mem_width` bytes per L1D access.

## Privileged architecture

### Privilege modes and traps

Machine, Supervisor and User modes. A program starts in M-mode; traps,
`MRET` and `SRET` move between modes. `medeleg` and `mideleg` delegate
exceptions and interrupts to S-mode. Interrupts are taken in the
specification's priority order (MEI, MSI, MTI, SEI, SSI, STI), at commit,
after everything already fetched has retired. `WFI` waits for an enabled
interrupt; when every hart waits and nothing is due, the simulator skips
the idle cycles.

`mstatus` implements MIE/SIE and their previous-state bits, MPP and SPP,
MPRV, SUM, MXR, TVM, TW, TSR, FS and VS. TVM makes `satp` accesses and
`SFENCE.VMA` illegal in S-mode, TSR makes `SRET` illegal in S-mode, and TW
makes `WFI` illegal in S-mode; `WFI` in U-mode is always illegal.

### CSRs

| Category | CSRs |
|----------|------|
| **Machine** | `mstatus`, `misa`, `medeleg`, `mideleg`, `mie`, `mip`, `mtvec`, `mscratch`, `mepc`, `mcause`, `mtval`, `mcounteren`, `mcountinhibit`, `menvcfg`, `mvendorid`, `marchid`, `mimpid`, `mhartid` |
| **Supervisor** | `sstatus`, `sie`, `sip`, `stvec`, `sscratch`, `sepc`, `scause`, `stval`, `satp`, `scounteren`, `senvcfg`, `stimecmp` |
| **Counters** | `cycle`, `time`, `instret`, `mcycle`, `minstret`; `mhpmcounter3`–`31` and `mhpmevent3`–`31` read zero (no event counts) |
| **Floating point** | `fflags`, `frm`, `fcsr` |
| **Vector** | `vstart`, `vxsat`, `vxrm`, `vcsr`, `vl`, `vtype`, `vlenb` |
| **PMP** | `pmpcfg0`, `pmpcfg2`, `pmpaddr0`–`pmpaddr15` |
| **Debug triggers** | `tselect`, `tdata1`, `tdata2`, `tdata3`, `tinfo`, `tcontrol` |

`mcountinhibit` stops `mcycle` and `minstret`. `menvcfg` holds STCE
(Sstc), ADUE (Svadu), and CBIE, CBCFE and CBZE, which gate the
cache-block operations in lower modes; `senvcfg` gates them for U-mode.

### Supervisor extensions

- **Sstc.** With `menvcfg.STCE` set, S-mode has its own timer compare,
  `stimecmp`, which raises the supervisor timer interrupt directly, so a
  kernel's timer needs no M-mode call.
- **Svade and Svadu.** By default a page whose A bit, or D bit on a
  store, is clear raises a page fault (Svade). With `svadu=True` and
  `menvcfg.ADUE` set, the page-table walker sets the bit itself (Svadu).
- **Sdtrig.** Two `mcontrol6` triggers match an exact execute, load or
  store address and raise a breakpoint exception, per mode, with
  `tcontrol.MTE` gating M-mode.

### Virtual memory

Sv39, Sv48 and Sv57, selected through `satp` and capped by
`Config(paging_mode_max=...)`; a mode beyond the cap leaves `satp` reading
Bare. A `satp` write takes effect at commit. `SFENCE.VMA` flushes the TLBs
at commit: all of them, those of one address, one ASID, or both, and never
a global mapping by ASID. Translation, the TLBs and the page-table walker
are described in [Memory Hierarchy](memory.md#virtual-memory).

### Physical memory protection

Sixteen PMP regions with TOR, NA4 and NAPOT matching and the lock bit.
PMP checks every S- and U-mode access, the page-table walker's PTE reads
included, and M-mode accesses to locked regions or with `mstatus.MPRV`
set.

## Conformance

| Suite | Result |
|-------|--------|
| [riscv-tests](https://github.com/riscv-software-src/riscv-tests) (`rv64ui`, `um`, `ua`, `uf`, `ud`, `uc`, `mi`, `si`) | 134 of 134 pass |
| [riscv-vector-tests](https://github.com/chipsalliance/riscv-vector-tests), cross-checked element by element against spike | 3023 of 3023 programs pass at VLEN 128 |
| Multi-core litmus and coherence programs | see [Multi-core](multicore.md) |
| Linux 6.6 through OpenSBI to a BusyBox shell | boots on one and eight harts |
