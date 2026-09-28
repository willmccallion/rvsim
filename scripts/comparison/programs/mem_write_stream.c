#include "common.h"
/* Stores streaming over 1 MiB: write misses and dirty evictions. */
#define N (1024 * 1024 / 8)
static u64 a[N];
int main(void) {
    for (int r = 0; r < 2; r++)
        for (int i = 0; i < N; i++) a[i] = i + r;
    sink = a[N - 1];
    return 0;
}
