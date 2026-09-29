#include "common.h"
/* Dependent loads 8 KiB + 8 bytes apart through 1 MiB: each one misses to
 * memory and waits for the one before. */
#define N (1024 * 1024 / 8)
#define STEP (1024 + 1)
static u64 next[N];
int main(void) {
    for (u64 i = 0; i < N; i++) next[i] = (i + STEP) % N;
    u64 p = 0;
    for (int i = 0; i < 20000; i++) p = next[p];
    sink = p;
    return 0;
}
