#include "bench.h"
#include "stdio.h"

// Demonstrates the benefit of register renaming in an out-of-order processor.
//
// Register renaming eliminates Write-After-Write (WAW) and Write-After-Read
// (WAR) hazards by assigning a unique physical register to each architectural
// register definition.  Without renaming, WAW hazards require the pipeline to
// serialise writes to the same architectural register.
//
// Loop A (WAW hazards): the architectural register 'a' is written multiple
//   times within one iteration (a=i+1 ... a=b+3 ... a=d-b).  An in-order
//   pipeline must complete each write before the next to preserve program
//   order; an OoO pipeline with renaming assigns distinct physical registers
//   to each write and retires them speculatively in parallel.
//
// Loop B (no WAW, distinct names): the same computation is spread across
//   five distinct architectural registers (a1..a5) so no WAW exists even
//   without renaming.  An in-order processor can execute this just as fast
//   as Loop A on a renamed OoO machine.
//
// Expected results:
//   In-order, no rename: Loop A slower than Loop B (WAW serialisation).
//   OoO with renaming:   Loop A ≈ Loop B (renaming removes the hazard).

volatile long sink = 0; // prevents dead-code elimination

int main() {
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Loop A: WAW hazards on architectural register 'a'
    // ------------------------------------------------------------------
    // 'a' is defined three times per iteration:
    //   a = i+1      (def 1)
    //   a = b+3      (def 2, uses 'b' from def below but not def 1 of 'a')
    //   a = d-b      (def 3, final value consumed by sink)
    // Without renaming, def 2 must wait for def 1 to complete (WAW on 'a'),
    // and def 3 must wait for def 2.  With renaming each def gets its own
    // physical register and can execute as soon as its true inputs are ready.
    start = read_cycles();
    for (int i = 0; i < 5000; i++) {
        long a, b, c, d;
        a = i + 1;   // def 1 of a
        b = i + 2;
        a = b + 3;   // def 2 of a (WAW with def 1); uses b
        c = a + 4;   // uses def 2 of a
        d = b + c;
        a = d - b;   // def 3 of a (WAW with def 2); this is the live value
        sink += a;   // consumes def 3
    }
    end = read_cycles();
    unsigned long waw_cycles = end - start;

    // ------------------------------------------------------------------
    // Loop B: no WAW — equivalent computation with distinct register names
    // ------------------------------------------------------------------
    // The same arithmetic is performed but each intermediate result is stored
    // in a distinct variable (a1..a5).  Even without register renaming there
    // are no WAW hazards; the pipeline can schedule freely based only on true
    // data dependences (RAW).
    start = read_cycles();
    for (int i = 0; i < 5000; i++) {
        long a1, a2, a3, a4, a5;
        a1 = i + 1;
        a2 = i + 2;
        a3 = i + 3;      // replaces the "a=b+3" intermediate
        a4 = a1 + a2;    // combines first two values (like c = a + 4 above)
        a5 = a3 + a4;    // final combination (equivalent final value)
        sink += a5;
    }
    end = read_cycles();
    unsigned long nowaw_cycles = end - start;

    printf("Loop A WAW    (5000 iters): %lu cycles  (%lu per iter)\n",
           waw_cycles,   waw_cycles   / 5000);
    printf("Loop B no-WAW (5000 iters): %lu cycles  (%lu per iter)\n",
           nowaw_cycles, nowaw_cycles / 5000);

    return 0;
}
