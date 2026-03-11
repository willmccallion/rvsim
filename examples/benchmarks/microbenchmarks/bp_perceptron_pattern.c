#include "bench.h"
#include "stdio.h"

// Demonstrates a branch whose outcome follows a long-history linear pattern
// that a perceptron (or TAGE-like) predictor can learn but simple 2-bit
// counters cannot.
//
// The branch outcome is determined by the parity of selected bits from a
// 64-bit Galois LFSR history register, masked with a fixed correlation
// pattern (0xAA55AA55AA55AA55).  This creates a linearly separable function
// of the branch history — exactly what perceptron predictors are designed to
// capture.
//
// Simulators with only a 2-bit or gshare predictor will show a high
// misprediction rate (~50%) and many wasted cycles.  A perceptron predictor
// trained over many iterations should converge to near-zero mispredictions.
//
// The LFSR advances each iteration, providing a pseudo-random but
// deterministic history stream with maximum-length period 2^64-1.

// Galois LFSR feedback polynomial: x^64 + x^63 + x^61 + x^60 + 1
// Tap mask: bits 63, 62, 60, 59  =>  0xD800000000000000
#define LFSR_POLY 0xD800000000000000UL

// Correlation mask: alternating nibbles of 0xAA and 0x55 taps select a
// long-range linear pattern in the history.
#define CORR_MASK 0xAA55AA55AA55AA55UL

volatile int sink = 0; // accumulates branch outcomes to prevent elimination

int main() {
    unsigned long history = 0xACE1ACEF12345678UL; // non-zero LFSR seed

    unsigned long start = read_cycles();

    for (int i = 0; i < 10000; i++) {
        // Advance LFSR: shift right, XOR feedback polynomial when LSB was 1
        // This produces one bit of pseudo-random history each iteration.
        int feedback = (int)(history & 1UL);
        history = (history >> 1) ^ (feedback ? LFSR_POLY : 0UL);

        // Branch outcome: parity of history & CORR_MASK.
        // Parity via XOR folding: a simple counter-based predictor cannot
        // learn this function; a perceptron predictor with sufficient history
        // length can.
        unsigned long v = history & CORR_MASK;
        v ^= v >> 32; v ^= v >> 16; v ^= v >> 8; v ^= v >> 4;
        v ^= v >> 2;  v ^= v >> 1;
        if (v & 1UL) {
            sink++; // taken path
        }
        // not-taken path falls through
    }

    unsigned long end = read_cycles();
    unsigned long total = end - start;

    printf("LFSR parity branch (10000 iters): %lu cycles\n", total);
    printf("Approx cycles per iter:           %lu\n", total / 10000);
    printf("sink (sanity check):              %d\n", sink);

    return 0;
}
