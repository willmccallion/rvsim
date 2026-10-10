#include "common.h"
/* A CSR write and read back on every iteration: what a serialising CSR
 * access costs the pipeline. mscratch holds no state the core acts on. */
int main(void) {
    u64 acc = 0;
    for (u64 i = 0; i < 20000; i++) {
        u64 v;
        __asm__ volatile("csrw mscratch, %1\n\tcsrr %0, mscratch" : "=r"(v) : "r"(i));
        acc += v;
    }
    sink = acc;
    return 0;
}
