/* The host side of a bare-metal program, as spike's proxy and Chipyard's
 * harness provide it: syscalls through an eight-word block handed over
 * `tohost`, answered through `fromhost`, and the exit through `tohost`. */
#include "htif.h"

volatile uint64_t tohost __attribute__((section(".tohost"), aligned(16)));
volatile uint64_t fromhost __attribute__((section(".tohost")));

/* tohost is one channel: harts take turns with it. */
static volatile uint32_t channel_lock;

static void lock_channel(void) {
    uint32_t held;
    do {
        __asm__ volatile("amoswap.w.aq %0, %2, (%1)" : "=r"(held) : "r"(&channel_lock), "r"(1) : "memory");
    } while (held);
}

static void unlock_channel(void) {
    __asm__ volatile("amoswap.w.rl zero, zero, (%0)" : : "r"(&channel_lock) : "memory");
}

uint64_t htif_syscall(uint64_t which, uint64_t arg0, uint64_t arg1, uint64_t arg2) {
    volatile uint64_t block[8] __attribute__((aligned(64)));
    block[0] = which;
    block[1] = arg0;
    block[2] = arg1;
    block[3] = arg2;
    lock_channel();
    __sync_synchronize();
    tohost = (uint64_t)(uintptr_t)block;
    while (fromhost == 0) {
    }
    fromhost = 0;
    __sync_synchronize();
    unlock_channel();
    return block[0];
}

void htif_exit(int code) {
    tohost = ((uint64_t)(unsigned)code << 1) | 1;
    for (;;) {
    }
}
