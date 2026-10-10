/* The host side of a bare-metal program, as spike's proxy and Chipyard's
 * harness provide it: syscalls through an eight-word block handed over
 * `tohost`, answered through `fromhost`, and the exit through `tohost`. */
#include "htif.h"

volatile uint64_t tohost __attribute__((section(".tohost"), aligned(16)));
volatile uint64_t fromhost __attribute__((section(".tohost")));

uint64_t htif_syscall(uint64_t which, uint64_t arg0, uint64_t arg1, uint64_t arg2) {
    volatile uint64_t block[8] __attribute__((aligned(64)));
    block[0] = which;
    block[1] = arg0;
    block[2] = arg1;
    block[3] = arg2;
    __sync_synchronize();
    tohost = (uint64_t)(uintptr_t)block;
    while (fromhost == 0) {
    }
    fromhost = 0;
    __sync_synchronize();
    return block[0];
}

void htif_exit(int code) {
    tohost = ((uint64_t)(unsigned)code << 1) | 1;
    for (;;) {
    }
}
