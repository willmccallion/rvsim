#include "common.h"
/* Short inner loops whose trip count the compiler cannot see: a
 * well-predicted taken branch every few instructions. */
static volatile int inner = 4;
int main(void) {
    u64 acc = 0;
    for (int i = 0; i < 30000; i++) {
        int n = inner;
        for (int j = 0; j < n; j++) acc += (u64)j * i;
    }
    sink = acc;
    return 0;
}
