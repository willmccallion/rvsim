#include "common.h"
/* A dependent multiply chain. */
int main(void) {
    u64 x = 3;
    for (int i = 0; i < 40000; i++) x = x * 2862933555777941757UL + 1;
    sink = x;
    return 0;
}
