#include "common.h"
/* Dependent double-precision divides: the FP divider's latency. */
static volatile double divisor = 1.000001;
int main(void) {
    double d = divisor;
    double x = 1.0e300;
    for (int i = 0; i < 8000; i++) x = x / d;
    sink = (u64)x;
    return 0;
}
