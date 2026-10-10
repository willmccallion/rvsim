#include "common.h"
/* A read of mscratch whose value the next instruction uses, on every
 * iteration: a CSR read's latency to a dependent. */
int main(void) {
    u64 acc = 0;
    for (u64 i = 0; i < 20000; i++) {
        u64 v;
        __asm__ volatile("csrr %0, mscratch\n\taddi %0, %0, 1" : "=r"(v));
        acc += v;
    }
    sink = acc;
    return 0;
}
