#include "chase.h"
/* 4 MiB of lines: misses the 512 KiB L2; the memory latency. */
#define N 65536
static unsigned char lines[N * CHASE_LINE + N * 8];
int main(void) {
    chase_build(lines, N, 5);
    sink = chase_run(lines, 16000);
    return 0;
}
