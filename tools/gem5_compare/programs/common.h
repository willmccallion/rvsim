/* Shared by the comparison programs: no I/O, only computation, so the
 * same ELF runs unchanged on rvsim and in gem5's syscall emulation. */
#ifndef COMPARE_COMMON_H
#define COMPARE_COMMON_H

typedef unsigned long u64;
typedef long i64;

/* Keeps a result live without any I/O. */
static volatile u64 sink;

static inline u64 lcg(u64 *state) {
    *state = *state * 6364136223846793005UL + 1442695040888963407UL;
    return *state >> 33;
}

#endif
