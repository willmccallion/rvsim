#include "common.h"
/* Zbb/Zbs/Zba-heavy integer work: popcount, clz, rotates, bit sets. */
#define N 16384
static u64 v[N];
int main(void) {
    u64 seed = 11;
    for (int i = 0; i < N; i++) v[i] = lcg(&seed) << 31 | lcg(&seed);
    u64 acc = 0;
    for (int r = 0; r < 4; r++)
        for (int i = 0; i < N; i++) {
            u64 x = v[i];
            acc += __builtin_popcountl(x) + __builtin_clzl(x | 1) + __builtin_ctzl(x | 1UL << 63);
            acc ^= (x << 13 | x >> 51) | (1UL << (x & 63));
        }
    sink = acc;
    return 0;
}
