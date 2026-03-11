#include "bench.h"
#include "stdio.h"

// Demonstrates store-to-load forwarding (also called store forwarding or
// memory bypassing) in the pipeline's load/store unit.
//
// When a STORE is followed closely by a LOAD to the SAME address, a
// processor with store-to-load forwarding can satisfy the load directly
// from the store buffer without waiting for the data to be written to and
// then re-read from the cache.  This avoids the full store→cache→load
// round-trip latency.
//
// Case A (same address, forwarding possible):
//   buf[0] = i;       <-- STORE to buf[0]
//   sink += buf[0];   <-- LOAD from buf[0] (same address, same iteration)
//   The load address matches the preceding store address.  A store-to-load
//   forwarding unit can provide the data in 1-2 cycles rather than the
//   full cache round-trip (~4 cycles for L1 hit).
//
// Case B (different address, no forwarding opportunity):
//   buf[0] = i;       <-- STORE to buf[0]
//   sink += buf[1];   <-- LOAD from buf[1] (different address)
//   The load address does not match any pending store, so the load must
//   wait for the normal L1 cache hit path.
//
// On a pipeline with store forwarding, Case A should be faster (or equal)
// to Case B.  On a pipeline without store forwarding, Case A may be SLOWER
// because the load must wait for the store to commit to cache before the
// load can proceed (store-load ordering stall).

volatile long sink = 0; // prevents dead-code elimination of load results

int main() {
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Case A: store then load from SAME address — forwarding candidate
    // ------------------------------------------------------------------
    start = read_cycles();
    for (int i = 0; i < 5000; i++) {
        long buf[2] = {100, 200}; // stack-local; fresh each iteration
        buf[0] = i;               // STORE to buf[0]
        sink += buf[0];           // LOAD from buf[0] — same address as store
    }
    end = read_cycles();
    unsigned long same_addr_cycles = end - start;

    // ------------------------------------------------------------------
    // Case B: store to buf[0], load from DIFFERENT address buf[1]
    // ------------------------------------------------------------------
    start = read_cycles();
    for (int i = 0; i < 5000; i++) {
        long buf[2] = {100, 200}; // stack-local; fresh each iteration
        buf[0] = i;               // STORE to buf[0]
        sink += buf[1];           // LOAD from buf[1] — different address
    }
    end = read_cycles();
    unsigned long diff_addr_cycles = end - start;

    printf("Case A same-addr  store->load fwd (5000 iters): %lu cycles\n",
           same_addr_cycles);
    printf("Case B diff-addr  no fwd possible  (5000 iters): %lu cycles\n",
           diff_addr_cycles);
    printf("sink (sanity): %ld\n", sink);

    return 0;
}
