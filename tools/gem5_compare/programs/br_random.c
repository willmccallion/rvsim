#include "common.h"
/* Data-dependent branches on pseudo-random bits: unpredictable. */
int main(void) {
    u64 s = 12345, acc = 0;
    for (int i = 0; i < 40000; i++) {
        if (lcg(&s) & 1) acc += i; else acc ^= i;
    }
    sink = acc;
    return 0;
}
