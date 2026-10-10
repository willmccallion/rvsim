#include "chase.h"
/* 4 KiB of lines: L1D hits; the load-to-use latency. */
#define N 64
static unsigned char lines[N * CHASE_LINE + N * 8];
int main(void) {
    chase_build(lines, N, 5);
    sink = chase_run(lines, 64000);
    return 0;
}
