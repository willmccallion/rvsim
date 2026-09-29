#include "common.h"
#include <riscv_vector.h>
/* Indexed loads from a 256 KiB table at pseudo-random offsets. */
#define TABLE (64 * 1024)
#define N 4096
static unsigned table[TABLE], idx[N];
int main(void) {
    u64 seed = 7;
    for (int i = 0; i < TABLE; i++) table[i] = i * 3;
    for (int i = 0; i < N; i++) idx[i] = (unsigned)(lcg(&seed) % TABLE) * sizeof(unsigned);
    u64 total = 0;
    for (int r = 0; r < 4; r++)
        for (long i = 0; i < N;) {
            long vl = __riscv_vsetvl_e32m1(N - i);
            vuint32m1_t off = __riscv_vle32_v_u32m1(&idx[i], vl);
            vuint32m1_t v = __riscv_vluxei32_v_u32m1(table, off, vl);
            vuint32m1_t s = __riscv_vredsum_vs_u32m1_u32m1(v, __riscv_vmv_s_x_u32m1(0, 1), vl);
            total += __riscv_vmv_x_s_u32m1_u32(s);
            i += vl;
        }
    sink = total;
    return 0;
}
