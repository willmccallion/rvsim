#include "common.h"
/* Loads whose results feed the next address: load-to-use latency. */
static u64 a[256];
int main(void) {
    for (int i = 0; i < 256; i++) a[i] = (i * 37 + 11) & 255;
    u64 p = 0;
    for (int i = 0; i < 60000; i++) p = a[p];
    sink = p;
    return 0;
}
