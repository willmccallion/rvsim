#include "common.h"
#include <riscv_vector.h>
/* y += a*x over 64 KiB double arrays with explicit RVV strip-mining. */
#define N 4096
static double x[N], y[N];
int main(void) {
    for (int i = 0; i < N; i++) { x[i] = i; y[i] = N - i; }
    for (int r = 0; r < 8; r++)
        for (long i = 0; i < N;) {
            long vl = __riscv_vsetvl_e64m2(N - i);
            vfloat64m2_t vx = __riscv_vle64_v_f64m2(&x[i], vl);
            vfloat64m2_t vy = __riscv_vle64_v_f64m2(&y[i], vl);
            __riscv_vse64_v_f64m2(&y[i], __riscv_vfmacc_vf_f64m2(vy, 1.5, vx, vl), vl);
            i += vl;
        }
    sink = (u64)y[N / 3];
    return 0;
}
