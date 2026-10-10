#include "common.h"
/* FENCE.I on every iteration: the instruction-fetch flush it costs. */
int main(void) {
    u64 acc = 0;
    for (u64 i = 0; i < 20000; i++) {
        __asm__ volatile("fence.i" ::: "memory");
        acc += i;
    }
    sink = acc;
    return 0;
}
