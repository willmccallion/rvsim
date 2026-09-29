#include "common.h"
/* A dependent floating-point add chain. */
int main(void) {
    double x = 0.5;
    for (int i = 0; i < 40000; i++) x = x + 1.000001;
    sink = (u64)x;
    return 0;
}
