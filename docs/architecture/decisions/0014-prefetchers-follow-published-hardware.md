# 14. Prefetchers follow the published hardware

**Context.** Every prefetcher lived in a cache and saw only the physical
address of each access. The stride prefetcher indexed its table by the
accessed line because no PC reached the cache, so a stride of a line or
more never trained, and the presets' L1D "stride" prefetchers behaved as
next-line prefetchers. Giving the cache the load's PC fixed the indexing
but exposed what a cache-side prefetcher cannot do: it does not know the
page size, cannot cross a page because the next physical page may belong
to anything, and never learns from stores, which leave the store buffer as
merged writes with no PC. Rounding strides up to a line and stopping at
4 KiB, as gem5 does, recovered the cycles but copied a simulator rather
than a core. Real cores answer these questions in documented ways:

- The Cortex-A72's load/store unit "includes a hardware prefetcher that is
  responsible for generating prefetches targeting both the L1D cache and
  L2 cache". Its load-side prefetcher "uses a hybrid mechanism which is
  based on both physical-address (PA) and virtual-address (VA)
  prefetching". With VA prefetch enabled, its reset state, it "use[s] VA
  in generating prefetches that can cross page boundaries"
  (`CPUACTLR_EL1[43]`); with it disabled, "prefetch is restricted to
  within the page boundary of the demand request". Its L2 load prefetch
  distance is "the number of requests by which the prefetch request to
  the L2, on a load stream, is ahead of the demand request stream", 16 to
  22, resetting to 22 (`CPUECTLR_EL1[33:32]`). "Prefetching on store
  accesses is managed by a PA based prefetcher and only prefetches to the
  L2 cache", fed by ReadUnique transactions (`CPUACTLR_EL1[42]`). (Arm,
  *Cortex-A72 MPCore Processor Technical Reference Manual* r0p3, §6.4.9,
  §4.3.66 and §4.3.67.)
- Intel's guide says of its hardware prefetcher that "it will not
  prefetch across a 4-KByte page boundary", while Gracemont's
  instruction-pointer stride prefetcher "works in the linear address
  space to cross page boundaries and start translations for TLB misses".
  (Intel, *64 and IA-32 Architectures Optimization Reference Manual*
  Vol. 1, 248966-049, §9.5.2 and §4.1.7.)
- SiFive has not published the P550's prefetchers.

**Decision.** Prefetchers sit where the hardware puts them and know what
it knows there.

- A **cache-side prefetcher** is a physical-address prefetcher. It never
  crosses the 4 KiB page of the access that triggered it, the smallest
  page a translation can map, and issues whole lines.
- The **load prefetcher** belongs to the load/store unit. It trains in
  memory1 on each load's PC and virtual address, keeps each confident
  stream `l1_lines` lines ahead in the L1D and `l2_lines` lines ahead in
  the L2, and at a page boundary either stops (at the trained page's real
  size, from the data TLB) or continues through the data TLB, dropping the
  prefetch on a TLB miss. A prefetch goes only to RAM the load could read.
- The **store prefetcher** belongs to the L1D. It is physical-address,
  triggered by the store misses that fetch for write permission, stays in
  the 4 KiB page, and fills only the L2.
- A **prefetch request** (`MemOp::Prefetch`) carries a prefetch from the
  load/store unit or the L1D to the level it fills; nothing answers it,
  and a cache drops it rather than give up its last MSHR.

Where a parameter is published (the A72's L2 distance and page policy) the
preset uses it; where it is not (the A72's L1 distance and table size, its
store run length, everything about the P550), the preset says so and takes
a cautious value.

**Consequences.** The presets prefetch the streams their hardware would:
strided loads of any stride, sequential loads at line granularity, and,
on the A72, store streams into the L2. A prefetcher that should stop at a
page now does, which costs a stream the first lines of each new page; one
that crosses needs the next page's translation in the data TLB. gem5's
cache-side `StridePrefetcher` with no MMU is closest to the cache-side
model here; the load/store-unit prefetcher has no gem5 counterpart, and
comparisons against gem5 use the cache-side one. The detection algorithms
themselves (a PC-indexed reference prediction table for loads, adjacent-
line runs for stores) are not documented for either core and are the
simplest designs that produce the documented behaviour.
