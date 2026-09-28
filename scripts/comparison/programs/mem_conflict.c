#include "common.h"
/* Sixteen lines that map to one L1 set, read round-robin. */
#define STRIDE (4096)
static volatile char a[STRIDE * 16];
int main(void) {
    u64 acc = 0;
    for (int r = 0; r < 4000; r++)
        for (int w = 0; w < 16; w++) acc += a[w * STRIDE];
    sink = acc;
    return 0;
}
