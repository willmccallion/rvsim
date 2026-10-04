# 13. Stores issue their address and data separately

**Context.** A store issued only when both its base register and its data
register were ready. A store whose data came from a long chain (a load
that missed, a divide) therefore withheld its address as well, and every
younger load that waits for older stores' addresses waited for that data.
Real out-of-order cores split a store into a store-address and a
store-data operation; gem5's O3 does not.

**Decision.** A plain scalar store whose base register is ready and whose
data is not issues its address half: it takes the store port and an
address unit, translates in memory1, and resolves its store-buffer slot's
address, which is what younger loads check against. Its issue-queue entry
stays and issues the data half when the value is ready; the data half
writes the slot's data and takes no unit or port. A store whose operands
are both ready issues whole. The store buffer's data stays optional until
it lands: a load that matches a store without data waits for it, and
commit retires a store only once its data is in. Atomics,
store-conditionals, cache-block operations and vector stores issue whole.

**Consequences.** Loads stop waiting for data that does not concern them,
which matters most for read-modify-write loops and stores fed by misses.
Store-heavy kernels run faster than in gem5 (`store_load_forward` by
20%), a difference that is gem5's
([decision 12](0012-the-model-follows-real-cores.md)). `lsq.split_stores`
counts the stores that split.
