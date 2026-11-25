/* BRANCHES taken jumps in a row, each SPACING bytes after the one before,
 * run ITERS times: the cycles per jump show how many branches the core's
 * branch target buffers hold and how fast each level delivers a target.
 * Built per setting by frontend_probe.py. */
#define STR(x) #x
#define XSTR(x) STR(x)

int main(void) {
    unsigned long iters = ITERS;
    __asm__ volatile(
        "   .option push\n"
        "   .option norvc\n"
        "   .balign 64\n"
        "1: .rept " XSTR(BRANCHES) "\n"
        "   j .+" XSTR(SPACING) "\n"
        "   .skip " XSTR(SPACING) " - 4, 0\n"
        "   .endr\n"
        "   addi %[iters], %[iters], -1\n"
        "   bnez %[iters], 1b\n"
        "   .option pop\n"
        : [iters] "+r"(iters)
        :
        : "memory");
    return 0;
}
