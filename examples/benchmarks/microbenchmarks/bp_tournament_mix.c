#include "bench.h"
#include "stdio.h"

// Demonstrates a tournament (hybrid) branch predictor handling a workload
// that mixes two distinct branch patterns in the same timed region:
//
//   Pattern A (inner accumulator loop): a simple counted loop 0..INNER-1.
//     This is highly biased "taken" and a local 2-bit counter predicts it
//     almost perfectly after two iterations.
//
//   Pattern B (data-dependent branch on data[]): the branch alternates
//     true/false with a period-7 pattern (4 positive, 3 negative values).
//     A global history predictor handles this better than a local counter
//     because the period does not align with powers of two.
//
// A tournament predictor maintains both a local and a global sub-predictor
// and a meta-predictor that selects between them per branch PC.  The total
// cycle count reflects how well the simulator's predictor arbitrates between
// these two patterns simultaneously.
//
// OUTER=50 outer iterations give the meta-predictor time to converge.

#define OUTER 50
#define INNER 100

// Period-7 data array: 4 positive then 3 negative entries, repeated.
// Initialised at global scope so the compiler cannot hoist it to compile time.
long data[512];

volatile long result = 0; // accumulates to prevent dead-code elimination

int main() {
    // Initialise data array: period 7 (indices 0-3 positive, 4-6 negative)
    for (int i = 0; i < 512; i++) {
        data[i] = ((i % 7) < 4) ? 1 : -1;
    }

    unsigned long start = read_cycles();

    for (int outer = 0; outer < OUTER; outer++) {

        // --- Pattern A: simple counted loop (local predictor friendly) ---
        // The loop-back branch is taken INNER-1 times then not-taken once.
        // A 2-bit saturating counter saturates to strongly-taken after iter 2
        // and mispredicts only at loop exit.
        for (int i = 0; i < INNER; i++) {
            result += i;
        }

        // --- Pattern B: data-dependent branch (global history friendly) ---
        // The branch alternates with period 7 driven by the data[] values.
        // A global history predictor that indexes its table with recent
        // outcomes can learn this repeating pattern.
        for (int i = 0; i < 512; i++) {
            if (data[i] > 0) {
                result += data[i]; // taken when data is positive
            } else {
                result -= data[i]; // taken when data is negative
            }
        }
    }

    unsigned long end = read_cycles();

    printf("Tournament mix (%d outer iters): %lu cycles\n", OUTER, end - start);
    printf("result (sanity check): %ld\n", result);

    return 0;
}
