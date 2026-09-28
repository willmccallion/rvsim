#include "common.h"
/* One read per 4 KiB page: every access a new line. */
#define BYTES (4 * 1024 * 1024)
static volatile char a[BYTES];
int main(void) {
    u64 acc = 0;
    for (int r = 0; r < 8; r++)
        for (int off = r * 64; off < BYTES; off += 4096) acc += a[off];
    sink = acc;
    return 0;
}
