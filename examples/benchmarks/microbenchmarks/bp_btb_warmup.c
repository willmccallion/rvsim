#include "bench.h"
#include "stdio.h"

// Demonstrates Branch Target Buffer (BTB) cold-start vs warm-hit cost for
// indirect calls through function pointers.
//
// Phase 1 (cold): each function pointer is called exactly once.  The BTB has
//   never seen these call sites, so every indirect jump suffers a full
//   misprediction penalty while the BTB learns the target address.
//
// Phase 2 (warm): the same eight function pointers are called 1000 times
//   each.  The BTB now holds all targets; indirect jumps resolve correctly
//   and the frontend can fetch the target without stalling.
//
// A significant drop in per-call cycles from Phase 1 to Phase 2 indicates a
// functioning BTB.  If costs are identical, the simulator may not yet model
// a BTB.

static long fn_a(long x) { return x + 1; }
static long fn_b(long x) { return x + 2; }
static long fn_c(long x) { return x + 3; }
static long fn_d(long x) { return x + 4; }
static long fn_e(long x) { return x + 5; }
static long fn_f(long x) { return x + 6; }
static long fn_g(long x) { return x + 7; }
static long fn_h(long x) { return x + 8; }

typedef long (*fn_t)(long);

// Table of function pointers — stored as global so the compiler cannot
// devirtualize the calls into direct branches.
fn_t table[8] = { fn_a, fn_b, fn_c, fn_d, fn_e, fn_f, fn_g, fn_h };

volatile long sink = 0; // prevent dead-code elimination of call results

int main() {
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Phase 1: cold — call each function pointer exactly once
    // ------------------------------------------------------------------
    // The BTB has no entry for any of these call sites on the first call.
    // Each indirect jump will be resolved late (or mispredicted) and the
    // target inserted into the BTB for future use.
    start = read_cycles();
    for (int f = 0; f < 8; f++) {
        sink += table[f](sink);
    }
    end = read_cycles();
    unsigned long cold_cycles = end - start;

    // ------------------------------------------------------------------
    // Phase 2: warm — call each function 1000 times
    // ------------------------------------------------------------------
    // After Phase 1 the BTB holds all eight targets.  Repeated calls to the
    // same indirect call sites should now hit the BTB and avoid mispredict
    // penalties.  We measure total cycles then divide to get per-call cost.
    start = read_cycles();
    for (int iter = 0; iter < 1000; iter++) {
        for (int f = 0; f < 8; f++) {
            sink += table[f](sink);
        }
    }
    end = read_cycles();
    unsigned long warm_cycles = end - start;
    // 8000 calls total in the warm phase
    unsigned long warm_per_call = warm_cycles / 8000;

    printf("Phase 1 cold  total cycles (8 calls):      %lu\n", cold_cycles);
    printf("Phase 2 warm  total cycles (8000 calls):   %lu\n", warm_cycles);
    printf("Phase 2 warm  per-call cycles:             %lu\n", warm_per_call);

    return 0;
}
