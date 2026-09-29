#include "common.h"
#include <riscv_vector.h>
/* Byte copies of 128 KiB at LMUL 8, the widest unit-stride accesses. */
#define BYTES (128 * 1024)
static unsigned char src[BYTES], dst[BYTES];
int main(void) {
    for (int i = 0; i < BYTES; i++) src[i] = (unsigned char)i;
    for (int r = 0; r < 4; r++)
        for (long i = 0; i < BYTES;) {
            long vl = __riscv_vsetvl_e8m8(BYTES - i);
            __riscv_vse8_v_u8m8(&dst[i], __riscv_vle8_v_u8m8(&src[i], vl), vl);
            i += vl;
        }
    sink = dst[BYTES / 7];
    return 0;
}
