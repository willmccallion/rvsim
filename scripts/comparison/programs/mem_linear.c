#include "common.h"
/* Sequential reads over 512 KiB, twice: misses every level, then the L2. */
#define N (512 * 1024 / 8)
static u64 a[N];
int main(void) {
    for (int i = 0; i < N; i++) a[i] = i;
    u64 acc = 0;
    for (int r = 0; r < 2; r++)
        for (int i = 0; i < N; i++) acc += a[i];
    sink = acc;
    return 0;
}
