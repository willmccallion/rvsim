#include "bench.h"
#include "stdio.h"

// Demonstrates the Return Address Stack (RAS) predictor under shallow vs deep
// call chains.
//
// A RAS is a small hardware stack that records return addresses pushed at
// CALL instructions and pops them at RET instructions, allowing the branch
// predictor to speculatively fetch the correct return target without a
// pipeline stall.
//
// Phase 1: depth=2  — only 2 nested CALL/RET pairs per iteration.
//   A 2-entry RAS (or larger) can handle this perfectly; mispredict rate
//   should be zero after the first iteration.
//
// Phase 2: depth=16 — 16 nested CALL/RET pairs.
//   If the simulator's RAS has fewer than 16 entries the deepest frames will
//   overflow the RAS, causing return-address mispredictions and extra stall
//   cycles.  A deeper RAS will show similar cycles/iter to Phase 1.
//
// Compare cycles/iter between phases to gauge effective RAS depth.

volatile long ras_sink = 0; // prevent elimination of recursive call results

// Recursive descent: depth=0 is the base case.
// The compiler should not tail-call-optimize this because the result of the
// recursive call is used in an addition, forcing a real CALL/RET pair.
long deep_call(int depth) {
    if (depth == 0) return 1;
    return 1 + deep_call(depth - 1);
}

int main() {
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Phase 1: shallow recursion (depth 2) — 2000 iterations
    // ------------------------------------------------------------------
    // Each call to deep_call(2) generates: call->call->ret->ret (2 levels).
    // A correctly functioning RAS should predict all returns correctly after
    // the very first iteration.
    start = read_cycles();
    for (int i = 0; i < 2000; i++) {
        ras_sink += deep_call(2);
    }
    end = read_cycles();
    unsigned long shallow_cycles = end - start;

    // ------------------------------------------------------------------
    // Phase 2: deep recursion (depth 16) — 2000 iterations
    // ------------------------------------------------------------------
    // Each call generates 16 nested CALL/RET pairs.  If the RAS overflows
    // (typically at 8 or 16 entries depending on implementation), the
    // outermost return targets will be mispredicted.
    start = read_cycles();
    for (int i = 0; i < 2000; i++) {
        ras_sink += deep_call(16);
    }
    end = read_cycles();
    unsigned long deep_cycles = end - start;

    printf("Phase 1 depth= 2  (2000 iters): %lu cycles  (%lu per iter)\n",
           shallow_cycles, shallow_cycles / 2000);
    printf("Phase 2 depth=16  (2000 iters): %lu cycles  (%lu per iter)\n",
           deep_cycles, deep_cycles / 2000);

    return 0;
}
