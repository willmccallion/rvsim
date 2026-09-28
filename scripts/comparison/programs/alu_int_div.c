#include "common.h"
/* A dependent divide chain. */
int main(void) {
    u64 x = ~0UL;
    for (int i = 0; i < 8000; i++) x = x / 3 + 0xFFFFFFFFFFFFUL;
    sink = x;
    return 0;
}
