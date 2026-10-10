#include "common.h"
/* Dependent 64-bit multiplies, eight unrolled: the multiplier's latency.
 * The barrier keeps the compiler from folding them into one multiply. */
#define MUL(p) do { (p) *= 0x9E3779B97F4A7C15UL; __asm__ volatile("" : "+r"(p)); } while (0)
int main(void) {
    u64 p = 3;
    for (int i = 0; i < 5000; i++) {
        MUL(p); MUL(p); MUL(p); MUL(p);
        MUL(p); MUL(p); MUL(p); MUL(p);
    }
    sink = p;
    return 0;
}
