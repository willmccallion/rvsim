#include "common.h"
/* Four independent multiply-add chains. */
int main(void) {
    double a = 1, b = 2, c = 3, d = 4;
    for (int i = 0; i < 20000; i++) {
        a = a * 0.999 + 1.0; b = b * 0.999 + 1.0; c = c * 0.999 + 1.0; d = d * 0.999 + 1.0;
    }
    sink = (u64)(a + b + c + d);
    return 0;
}
