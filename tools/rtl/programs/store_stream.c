#include "common.h"
/* A store to a new cache line on every iteration, over a buffer larger
 * than the L2: what a store miss costs the pipeline. */
#define LINES (3 * 1024 * 1024 / 64)
static u64 buffer[LINES * 8] __attribute__((aligned(64)));
int main(void) {
    for (u64 i = 0; i < LINES; i++) {
        __asm__ volatile("sd %0, 0(%1)" ::"r"(i), "r"(&buffer[i * 8]) : "memory");
    }
    sink = buffer[8 * 1000];
    return 0;
}
