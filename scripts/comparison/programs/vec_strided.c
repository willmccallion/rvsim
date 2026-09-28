#include "common.h"
#include <riscv_vector.h>
/* Column sums of a 256x256 int matrix: strided vector loads, 1 KiB apart. */
#define N 256
static int m[N][N];
int main(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) m[i][j] = i ^ j;
    long total = 0;
    for (int j = 0; j < N; j++) {
        vint32m1_t acc = __riscv_vmv_s_x_i32m1(0, 1);
        for (long i = 0; i < N;) {
            long vl = __riscv_vsetvl_e32m1(N - i);
            vint32m1_t col = __riscv_vlse32_v_i32m1(&m[i][j], N * sizeof(int), vl);
            acc = __riscv_vredsum_vs_i32m1_i32m1(col, acc, vl);
            i += vl;
        }
        total += __riscv_vmv_x_s_i32m1_i32(acc);
    }
    sink = (u64)total;
    return 0;
}
