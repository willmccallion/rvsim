#include "common.h"
/* Swaps of random pairs through 1 MiB: independent load and store misses. */
#define N (1024 * 1024 / 8)
static u64 v[N];
int main(void) {
    u64 s = 99;
    for (u64 i = N - 1; i > N / 2; i--) {
        u64 j = lcg(&s) % i;
        u64 t = v[i]; v[i] = v[j]; v[j] = t;
    }
    sink = v[N / 3];
    return 0;
}
