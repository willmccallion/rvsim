#include "bench.h"
#include "stdio.h"

// Demonstrates the effect of Miss Status Holding Registers (MSHRs) on
// strided memory access throughput.
//
// MSHRs (also called "lockup-free cache" entries) allow the cache to track
// multiple outstanding cache misses simultaneously.  When MSHR count > 1,
// the processor can issue a new load while a previous miss is still being
// serviced by the memory system, effectively overlapping memory latency.
//
// This benchmark accesses data[i * STRIDE] for i in 0..N-1.
//   STRIDE = 128 longs = 1024 bytes = 16 cache lines apart.
// Each access lands on a different cache line, so every access is a miss
// (the array is N*STRIDE*8 = 512*128*8 = 524288 bytes = 512 KiB, larger
// than a typical L1 cache).
//
// With 1 MSHR:  each miss blocks until the line returns before the next
//   load is issued.  Total time ≈ N * memory_latency.
//
// With k MSHRs: up to k misses can be in-flight simultaneously.  If the
//   out-of-order window is large enough to expose them, total time ≈
//   (N / k) * memory_latency (capped by memory bandwidth).
//
// Comparing the measured cycles to N * expected_L1_miss_latency reveals
// the effective MSHR parallelism the simulator provides.

#define N      512   // number of stride-separated accesses
#define STRIDE 128   // stride in units of long (= 1 KiB between accesses)

// Array large enough that every strided element misses in L1.
// Total size: N * STRIDE * sizeof(long) = 512 KiB.
long data[N * STRIDE];

volatile long sink; // prevents the load-sum from being optimised away

int main() {
    // Initialise array (untimed) so values are defined.
    // Touch every accessed element to bring them into DRAM row buffers but
    // NOT into cache (the init itself flushes cache lines through the array).
    for (int i = 0; i < N; i++) {
        data[i * STRIDE] = i + 1;
    }

    // ------------------------------------------------------------------
    // Timed phase: strided loads — exposes MSHR parallelism
    // ------------------------------------------------------------------
    // The CPU must load N elements, each separated by STRIDE*8 bytes.
    // An in-order CPU with 1 MSHR serialises all misses.
    // An OoO CPU with multiple MSHRs can overlap miss service.
    long s = 0;
    unsigned long start = read_cycles();
    for (int i = 0; i < N; i++) {
        s += data[i * STRIDE]; // each access targets a distinct cache line
    }
    unsigned long end = read_cycles();

    sink = s; // commit result through volatile store

    unsigned long total = end - start;
    printf("Strided load %d elements (stride=%d longs): %lu cycles\n",
           N, STRIDE, total);
    printf("Cycles per load: %lu\n", total / N);
    printf("sink (sanity): %ld\n", sink);

    return 0;
}
