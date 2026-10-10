#include "common.h"
/* An ECALL into a handler that returns past it, on every iteration: the
 * trap entry and MRET cost. */
__asm__(
    ".section .text\n"
    ".align 2\n"
    "trap_handler:\n"
    "    csrr t0, mepc\n"
    "    addi t0, t0, 4\n"
    "    csrw mepc, t0\n"
    "    mret\n");
extern void trap_handler(void);
int main(void) {
    __asm__ volatile("csrw mtvec, %0" ::"r"(trap_handler));
    u64 acc = 0;
    for (u64 i = 0; i < 20000; i++) {
        __asm__ volatile("ecall" ::: "memory");
        acc += i;
    }
    sink = acc;
    return 0;
}
