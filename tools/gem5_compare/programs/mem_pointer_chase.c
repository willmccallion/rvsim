#include "common.h"
/* A random cycle through 1 MiB: every load waits for the one before. */
#define N (1024 * 1024 / 8)
static u64 next[N];
int main(void) {
    u64 s = 99;
    for (u64 i = 0; i < N; i++) next[i] = i;
    for (u64 i = N - 1; i > 0; i--) {
        u64 j = lcg(&s) % i;
        u64 t = next[i]; next[i] = next[j]; next[j] = t;
    }
    u64 p = 0;
    for (int i = 0; i < 40000; i++) p = next[p];
    sink = p;
    return 0;
}
