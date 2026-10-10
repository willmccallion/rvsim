#include "chase.h"
/* 256 KiB of lines: misses the 32 KiB L1D, hits the 512 KiB L2. */
#define N 4096
static unsigned char lines[N * CHASE_LINE + N * 8];
int main(void) {
    chase_build(lines, N, 5);
    sink = chase_run(lines, 32000);
    return 0;
}
