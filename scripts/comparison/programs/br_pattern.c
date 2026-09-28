#include "common.h"
/* A branch with a period-7 pattern and one with a period-3 pattern:
 * learnable from history. */
int main(void) {
    u64 acc = 0;
    for (int i = 0; i < 60000; i++) {
        if ((i % 7) < 3) acc += i;
        if ((i % 3) == 0) acc ^= acc >> 3;
    }
    sink = acc;
    return 0;
}
