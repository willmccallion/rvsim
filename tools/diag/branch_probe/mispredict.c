/* A loop over N bytes, PASSES times, whose one conditional branch is taken
 * when the byte is set: random bits when RANDOM is 1, so the branch
 * mispredicts about half the time, or all clear when it is 0, so it never
 * does. The bytes are drawn the same way in both builds, so the two differ
 * only in what the branch sees, and the cycles between them over the
 * mispredictions between them is what each misprediction costs. The loop
 * is written in assembly so the compiler keeps the branch. Built per
 * setting by mispredict_probe.py. */
typedef unsigned long u64;

static unsigned char bits[N];
static u64 volatile sink;
static inline u64 lcg(u64 *s) {
    *s = *s * 6364136223846793005UL + 1442695040888963407UL;
    return *s >> 33;
}
int main(void) {
    u64 s = 99;
    for (u64 i = 0; i < N; i++) bits[i] = (lcg(&s) & 1) & RANDOM;
    u64 taken = 0;
    for (u64 pass = 0; pass < PASSES; pass++) {
        const unsigned char *p = bits, *end = bits + N;
        __asm__ volatile(
            "1: lbu t0, 0(%[p])\n"
            "   beqz t0, 2f\n"
            "   addi %[taken], %[taken], 1\n"
            "2: addi %[p], %[p], 1\n"
            "   bne %[p], %[end], 1b\n"
            : [p] "+r"(p), [taken] "+r"(taken)
            : [end] "r"(end)
            : "t0", "memory");
    }
    sink = taken;
    return 0;
}
