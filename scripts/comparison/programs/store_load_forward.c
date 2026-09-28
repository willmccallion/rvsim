#include "common.h"
/* A store read back at once: store-to-load forwarding. */
static volatile u64 slot[8];
int main(void) {
    u64 acc = 0;
    for (int i = 0; i < 30000; i++) {
        slot[i & 7] = acc + i;
        acc += slot[i & 7];
    }
    sink = acc;
    return 0;
}
