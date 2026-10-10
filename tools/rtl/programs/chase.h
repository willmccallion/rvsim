/* A pointer chase through a random cycle of `n` cache lines: each load's
 * address is the previous load's value, eight loads unrolled so a 1-wide
 * core shows the load-to-use latency of the level the lines live in. */
#ifndef CHASE_H
#define CHASE_H
#include "common.h"

#define CHASE_LINE 64

static inline void chase_build(unsigned char *lines, u64 n, u64 seed) {
    u64 *order = (u64 *)(lines + n * CHASE_LINE);
    for (u64 i = 0; i < n; i++) order[i] = i;
    for (u64 i = n - 1; i > 0; i--) {
        u64 j = lcg(&seed) % i;
        u64 t = order[i]; order[i] = order[j]; order[j] = t;
    }
    for (u64 i = 0; i < n; i++) {
        *(u64 *)(lines + order[i] * CHASE_LINE) =
            (u64)(lines + order[(i + 1) % n] * CHASE_LINE);
    }
}

static inline u64 chase_run(unsigned char *lines, int loads) {
    u64 p = (u64)lines;
    for (int i = 0; i < loads / 8; i++) {
        p = *(u64 *)p; p = *(u64 *)p; p = *(u64 *)p; p = *(u64 *)p;
        p = *(u64 *)p; p = *(u64 *)p; p = *(u64 *)p; p = *(u64 *)p;
    }
    return p;
}

#endif
