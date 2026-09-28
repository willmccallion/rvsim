#include "common.h"
#include <riscv_vector.h>
/* Integer dot product: element-wise multiply, then a sum reduction. */
#define N 8192
static int a[N], b[N];
int main(void) {
    for (int i = 0; i < N; i++) { a[i] = i & 255; b[i] = (i * 7) & 255; }
    long total = 0;
    for (int r = 0; r < 4; r++) {
        vint32m1_t acc = __riscv_vmv_s_x_i32m1(0, 1);
        for (long i = 0; i < N;) {
            long vl = __riscv_vsetvl_e32m1(N - i);
            vint32m1_t p = __riscv_vmul_vv_i32m1(__riscv_vle32_v_i32m1(&a[i], vl),
                                                 __riscv_vle32_v_i32m1(&b[i], vl), vl);
            acc = __riscv_vredsum_vs_i32m1_i32m1(p, acc, vl);
            i += vl;
        }
        total += __riscv_vmv_x_s_i32m1_i32(acc);
    }
    sink = (u64)total;
    return 0;
}
