/* Henry Wong's structure-size test: two independent pointer chases, each
 * load a cold miss to memory, with FILLERS instructions of one kind after
 * each load. While a chase's load, the fillers and the other chase's load
 * all fit in the structure the fillers use up, the two misses overlap and
 * an iteration takes one memory latency; once they do not, the second
 * waits for the first and an iteration takes two. Built per filler count
 * and kind by structure_probe.py, which also writes the chains into
 * window_chains.bin in the build directory: 8-byte offsets from the file's
 * start, the first two to each chain's first node and each node's to the
 * next, every node on a line of its own in random order. The loader places
 * the file in memory without the core touching it, so every load misses.
 *
 * KIND 1: nop (reorder buffer)
 * KIND 2: an integer add (integer rename registers)
 * KIND 3: an FP add (FP rename registers)
 * KIND 4: a load that hits the L1D (load queue)
 * KIND 5: a store (store queue)
 * KIND 6: an add waiting on the miss before it (scheduler) */
__asm__(".section .data\n.balign 4096\nchain: .incbin \"window_chains.bin\"\n.previous");

#if KIND == 1
#define FILL_A "nop"
#define FILL_B "nop"
#elif KIND == 2
#define FILL_A "add t3, t4, t5"
#define FILL_B "add t3, t4, t5"
#elif KIND == 3
#define FILL_A "fadd.d ft0, ft1, ft2"
#define FILL_B "fadd.d ft0, ft1, ft2"
#elif KIND == 4
#define FILL_A "ld t3, 0(sp)"
#define FILL_B "ld t3, 0(sp)"
#elif KIND == 5
#define FILL_A "sd t4, 0(sp)"
#define FILL_B "sd t4, 0(sp)"
#elif KIND == 6
#define FILL_A "add t3, t0, t0"
#define FILL_B "add t3, t1, t1"
#endif

#define STR(x) #x
#define XSTR(x) STR(x)

int main(void) {
    unsigned long iters = ITERS;
    __asm__ volatile(
        "   la t6, chain\n"
        "   ld a4, 0(t6)\n"
        "   add a4, t6, a4\n"
        "   ld a5, 8(t6)\n"
        "   add a5, t6, a5\n"
        "   li t4, 1\n"
        "   li t5, 2\n"
        "   fcvt.d.l ft1, t4\n"
        "   fcvt.d.l ft2, t5\n"
        "1: ld t0, 0(a4)\n"
        "   add a4, t6, t0\n"
        "   .rept " XSTR(FILLERS) "\n " FILL_A "\n .endr\n"
        "   ld t1, 0(a5)\n"
        "   add a5, t6, t1\n"
        "   .rept " XSTR(FILLERS) "\n " FILL_B "\n .endr\n"
        "   addi %[iters], %[iters], -1\n"
        "   bnez %[iters], 1b\n"
        : [iters] "+r"(iters)
        :
        : "t0", "t1", "t3", "t4", "t5", "t6", "a4", "a5", "ft0", "ft1", "ft2",
          "memory");
    return 0;
}
