#include "common.h"
/* Dependent 64-bit divides by a divisor the compiler cannot see, so each
 * is a real divu: the divider's latency on operands of about 48 bits. */
static volatile u64 divisor = 3;
int main(void) {
    u64 d = divisor;
    u64 x = ~0UL;
    for (int i = 0; i < 8000; i++) x = x / d + 0xFFFFFFFFFFFFUL;
    sink = x;
    return 0;
}
