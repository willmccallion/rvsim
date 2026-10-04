/* A dependent pointer chase over FOOTPRINT bytes for STEPS steps: each
 * element holds the address of the next, so every load waits for the one
 * before and nothing else, and cycles per step is the load-to-use latency
 * of the level the footprint fits in. Built per footprint and step count
 * by latency_probe.py. */
typedef unsigned long u64;
#define N (FOOTPRINT / 8)
static u64 *next[N];
static u64 *volatile sink;
static inline u64 lcg(u64 *s) {
    *s = *s * 6364136223846793005UL + 1442695040888963407UL;
    return *s >> 33;
}
int main(void) {
    u64 s = 99;
    for (u64 i = 0; i < N; i++) next[i] = (u64 *)&next[i];
    for (u64 i = N - 1; i > 0; i--) {
        u64 j = lcg(&s) % i;
        u64 *t = next[i]; next[i] = next[j]; next[j] = t;
    }
    u64 *p = next[0];
    for (u64 i = 0; i < STEPS; i++) p = (u64 *)*p;
    sink = p;
    return 0;
}
