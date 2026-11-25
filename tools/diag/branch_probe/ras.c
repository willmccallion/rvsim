/* A call chain DEPTH frames deep that then returns all the way, run ITERS
 * times: once the chain is deeper than the return stack, returns
 * mispredict and the cycles per call and return pair jump. The frames are
 * laid out one after another, each calling the next, so no conditional
 * branch ends the chain for a direction predictor to miss. Built per
 * setting by frontend_probe.py. */
#define STR(x) #x
#define XSTR(x) STR(x)

int main(void) {
    unsigned long iters = ITERS;
    __asm__ volatile(
        "   .option push\n"
        "   .option norvc\n"
        "   j 3f\n"
        /* Each frame is 24 bytes; its jal reaches the next frame's start. */
        "2: .rept " XSTR(DEPTH) " - 1\n"
        "   addi sp, sp, -16\n"
        "   sd ra, 0(sp)\n"
        "   jal ra, .+16\n"
        "   ld ra, 0(sp)\n"
        "   addi sp, sp, 16\n"
        "   ret\n"
        "   .endr\n"
        "   ret\n"
        "3: call 2b\n"
        "   addi %[iters], %[iters], -1\n"
        "   bnez %[iters], 3b\n"
        "   .option pop\n"
        : [iters] "+r"(iters)
        :
        : "ra", "memory");
    return 0;
}
