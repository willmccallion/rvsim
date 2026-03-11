#include "bench.h"
#include "stdio.h"

// Demonstrates 2-bit saturating counter branch predictor warm-up behaviour.
//
// A 2-bit saturating counter starts in state "weakly not-taken" (or similar)
// and requires two consecutive taken outcomes before it strongly predicts
// "taken".  This means the very first few iterations of a new loop suffer
// mispredictions while the counter saturates.
//
// Phase 1: only 10 iterations — the counter may never fully saturate, so a
//   larger fraction of iterations incur misprediction penalties.  The
//   cycles-per-iteration cost is therefore higher (dominated by mispredict
//   recovery pipeline flushes).
//
// Phase 2: 1000 iterations — the counter saturates quickly (after 1-2
//   mispredicts) and remains in the strongly-taken state for the remainder.
//   The per-iteration cost drops to the steady-state prediction-hit cost.
//
// The ratio (phase1 cycles/iter) / (phase2 cycles/iter) reveals how many
// iterations the predictor "wastes" warming up.

volatile int counter = 0; // global so the loop body is not optimised away

int main() {
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Phase 1: short loop — 10 iterations, predictor barely warms up
    // ------------------------------------------------------------------
    // Reset counter so both phases start from the same logical state.
    counter = 0;
    start = read_cycles();
    for (int i = 0; i < 10; i++) {
        counter += i; // loop-back branch: predictor learns "taken" slowly
    }
    end = read_cycles();
    unsigned long short_cycles = end - start;

    // ------------------------------------------------------------------
    // Phase 2: long loop — 1000 iterations, predictor saturates early
    // ------------------------------------------------------------------
    counter = 0;
    start = read_cycles();
    for (int i = 0; i < 1000; i++) {
        counter += i; // same branch pattern; predictor saturates by iter 2
    }
    end = read_cycles();
    unsigned long long_cycles = end - start;

    // Per-iteration costs (fixed-point, scaled by 100 for two decimal places)
    unsigned long short_per100 = (short_cycles * 100) / 10;
    unsigned long long_per100  = (long_cycles  * 100) / 1000;

    printf("Phase 1 short loop (10 iters):   %lu cycles  (%lu.%02lu per iter)\n",
           short_cycles, short_per100 / 100, short_per100 % 100);
    printf("Phase 2 long  loop (1000 iters): %lu cycles  (%lu.%02lu per iter)\n",
           long_cycles,  long_per100  / 100, long_per100  % 100);

    return 0;
}
