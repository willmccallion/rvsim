#include "common.h"
/* A write to mie on every iteration: what a CSR write the core flushes
 * after costs. Rocket flushes after a write to any CSR but the scratch,
 * epc, cause and tval ones; no interrupt is enabled or pending here. */
int main(void) {
    u64 acc = 0;
    for (u64 i = 0; i < 20000; i++) {
        __asm__ volatile("csrw mie, zero" ::: "memory");
        acc += i;
    }
    sink = acc;
    return 0;
}
