#ifndef RVSIM_H
#define RVSIM_H

// Pseudo-operations: requests the program makes of the simulator with
// gem5's m5op instruction (opcode 0x7b, the operation in bits 31:25, its
// arguments in a0 and a1), performed when the instruction retires, in a
// bare-metal image, a user program under SE mode or a program inside a
// Linux guest alike. The numbers are gem5's, so a binary instrumented
// with gem5's libm5 runs unchanged. See docs/pseudo-ops.md.

#define RVSIM_OP_FAIL 0x22
#define RVSIM_OP_RESET_STATS 0x40
#define RVSIM_OP_DUMP_STATS 0x41
#define RVSIM_OP_WORK_BEGIN 0x5a

#define RVSIM_STR_(x) #x
#define RVSIM_STR(x) RVSIM_STR_(x)
#define RVSIM_PSEUDO_OP(op, arg0, arg1)                                        \
  do {                                                                         \
    register unsigned long rvsim_a0_ __asm__("a0") = (arg0);                   \
    register unsigned long rvsim_a1_ __asm__("a1") = (arg1);                   \
    __asm__ volatile(".word 0x7b | (" RVSIM_STR(op) " << 25)"                  \
                     :                                                         \
                     : "r"(rvsim_a0_), "r"(rvsim_a1_)                          \
                     : "memory");                                              \
  } while (0)

// Keeps a snapshot of the stats labelled `label`; the host subtracts two
// snapshots to get the stats of the region between them.
static inline void rvsim_dump_stats(unsigned long label) {
  RVSIM_PSEUDO_OP(RVSIM_OP_DUMP_STATS, label, 0);
}

// Zeroes the stats.
static inline void rvsim_reset_stats(void) {
  RVSIM_PSEUDO_OP(RVSIM_OP_RESET_STATS, 0, 0);
}

// Stops the host's run here, labelled `label`, for it to save, switch
// configuration or measure from this point.
static inline void rvsim_break(unsigned long label) {
  RVSIM_PSEUDO_OP(RVSIM_OP_WORK_BEGIN, label, 0);
}

// Ends the simulation with exit code `code`.
static inline void rvsim_exit(unsigned long code) {
  RVSIM_PSEUDO_OP(RVSIM_OP_FAIL, 0, code);
}

#endif
