#include "common.h"
/* Indirect calls through a table, target chosen by a repeating sequence. */
__attribute__((noinline)) static u64 f0(u64 x) { return x + 1; }
__attribute__((noinline)) static u64 f1(u64 x) { return x ^ 0x55; }
__attribute__((noinline)) static u64 f2(u64 x) { return x * 3; }
__attribute__((noinline)) static u64 f3(u64 x) { return x >> 1; }
static u64 (*const table[4])(u64) = {f0, f1, f2, f3};
int main(void) {
    u64 acc = 1;
    for (int i = 0; i < 30000; i++) acc = table[(i * 5 + (i >> 2)) & 3](acc);
    sink = acc;
    return 0;
}
