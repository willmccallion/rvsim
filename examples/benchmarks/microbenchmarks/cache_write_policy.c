#include "bench.h"
#include "stdio.h"

// Demonstrates the cost difference between cache write operations and
// subsequent read-back, exposing write-policy behaviour (write-back vs
// write-through) and write-allocate vs no-write-allocate policies.
//
// Phase 1 (timed writes): sequentially write every element of wbuf[].
//   - Write-allocate + write-back: the cache fetches each cache line on the
//     first write miss, then marks it dirty.  Subsequent writes to the same
//     line hit the cache.  Write traffic stays in the cache until eviction.
//   - Write-through: every store propagates immediately to the next level,
//     increasing memory bus traffic and potentially stalling the store buffer.
//   - No-write-allocate: missed stores bypass the cache and go directly to
//     memory, so the data is cold for Phase 2 reads.
//
// Phase 2 (timed reads): read every element of wbuf[] back into checksum.
//   - If write-allocate: lines are already in cache from Phase 1 writes;
//     reads should hit, giving a lower cycle count than Phase 1.
//   - If no-write-allocate: lines are cold; reads must fetch from memory,
//     giving a similar (or higher) cost to Phase 1.
//
// SIZE is chosen so the array fits comfortably in a typical L1 data cache
// (8192 longs = 64 KiB), isolating write-policy effects from capacity misses.

#define SIZE 8192

long wbuf[SIZE]; // write target — global to ensure it lives in BSS/data
long rbuf[SIZE]; // spare buffer (unused in timing, keeps linker happy)

volatile long checksum = 0; // read-back accumulator; volatile prevents hoisting

int main() {
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Phase 1: write entire wbuf sequentially (timed)
    // ------------------------------------------------------------------
    // All stores go to the same array; the access pattern is fully
    // sequential so hardware prefetchers will engage quickly.  The key
    // variable is what happens at a write miss: allocate or bypass?
    start = read_cycles();
    for (int i = 0; i < SIZE; i++) {
        wbuf[i] = (long)i * 3 + 1; // non-trivial value prevents compile-time fold
    }
    end = read_cycles();
    unsigned long write_cycles = end - start;

    // ------------------------------------------------------------------
    // Phase 2: read wbuf back sequentially (timed)
    // ------------------------------------------------------------------
    // If Phase 1 used write-allocate, every cache line is already loaded;
    // reads should be cache hits throughout.  If Phase 1 bypassed the
    // cache, reads will be cold misses and take longer.
    start = read_cycles();
    for (int i = 0; i < SIZE; i++) {
        checksum += wbuf[i]; // accumulate into volatile to force each load
    }
    end = read_cycles();
    unsigned long read_cycles_val = end - start;

    printf("Phase 1 write %d longs: %lu cycles\n", SIZE, write_cycles);
    printf("Phase 2 read  %d longs: %lu cycles\n", SIZE, read_cycles_val);
    printf("checksum (sanity): %ld\n", checksum);

    return 0;
}
