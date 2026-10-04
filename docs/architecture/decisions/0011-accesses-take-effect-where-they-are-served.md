# 11. Accesses take effect where they are served

**Context.** RAM was an `Arc<DramBuffer>` the memory controllers and the
virtio device shared, with raw-pointer `RamRegion` copies in the bus and
the memory image, and `unsafe impl Send + Sync` on the types that held
them. Loads read RAM and stores wrote it from the pipeline, at times
unrelated to when the cache hierarchy served them, so a load in a
multi-core run could see another hart's store before the coherence fabric
had delivered it.

**Decision.** Caches hold tags and coherence state only. One memory image,
`sim::memory::GlobalMemory`, owns RAM, the LR/SC reservations and a log of
RAM writes, and every event-driven component receives it. An access takes
effect at its perform point, in the component that serves it: an L1D hit
as it arrives, a miss when its fill returns, an access no cache serves at
the memory controller. Line fills and writebacks between levels move
permission and timing only. Each write names who made it (a hart's store,
SC, AMO or vector store; a device's DMA; the host), and AMOs and
store-conditionals are performed in the cache as one access.

**Consequences.** There is one copy of every byte, so a coherence or
timing bug can make an access happen at the wrong time but cannot make two
copies of a line disagree. A load sees exactly the stores performed before
it, which gives multi-core runs real memory-ordering behaviour, and the
write log lets the load queue detect a younger load that read a line
another hart has since written. RAM's accessors are borrow-checked slices,
and the blanket `Send`/`Sync` impls are gone. A host probe of memory
reads the image, so it does not see a store until the cache has performed
it.
