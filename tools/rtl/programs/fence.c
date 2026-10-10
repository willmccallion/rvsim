#include "common.h"
/* Stores followed by a FENCE on every iteration: the store-buffer drain. */
static u64 a[64];
int main(void) {
    for (u64 i = 0; i < 20000; i++) {
        a[i & 63] = i;
        a[(i + 1) & 63] = i + 1;
        __asm__ volatile("fence" ::: "memory");
    }
    sink = a[3];
    return 0;
}
