#ifndef RVSIM_H
#define RVSIM_H

// Pseudo-operations: requests the program makes of the simulator with
// gem5's m5op instruction (opcode 0x7b, the operation in bits 31:25, its
// arguments in a0 to a3 and, for the file operations, its result in a0),
// performed when the instruction retires, in a bare-metal image, a user
// program under SE mode or a program inside a Linux guest alike. The
// numbers are gem5's, so a binary instrumented with gem5's libm5 runs
// unchanged. See docs/pseudo-ops.md.

#define RVSIM_OP_RPNS 0x07
#define RVSIM_OP_FAIL 0x22
#define RVSIM_OP_RESET_STATS 0x40
#define RVSIM_OP_DUMP_STATS 0x41
#define RVSIM_OP_WRITE_FILE 0x4f
#define RVSIM_OP_READ_FILE 0x50
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

// The simulated time in nanoseconds, as gem5's m5_rpns reports it.
static inline unsigned long rvsim_time_ns(void) {
  register unsigned long rvsim_a0_ __asm__("a0");
  __asm__ volatile(".word 0x7b | (" RVSIM_STR(RVSIM_OP_RPNS) " << 25)"
                   : "=r"(rvsim_a0_)
                   :
                   : "memory");
  return rvsim_a0_;
}

// Copies up to `len` bytes of the host's script file, from `offset`, into
// `buf`, and returns how many: 0 at the end of the file, or without one,
// and a negated errno when `buf` is not mapped.
static inline long rvsim_read_file(void *buf, unsigned long len,
                                   unsigned long offset) {
  register unsigned long rvsim_a0_ __asm__("a0") = (unsigned long)buf;
  register unsigned long rvsim_a1_ __asm__("a1") = len;
  register unsigned long rvsim_a2_ __asm__("a2") = offset;
  __asm__ volatile(".word 0x7b | (" RVSIM_STR(RVSIM_OP_READ_FILE) " << 25)"
                   : "+r"(rvsim_a0_)
                   : "r"(rvsim_a1_), "r"(rvsim_a2_)
                   : "memory");
  return (long)rvsim_a0_;
}

// Writes `len` bytes of `buf` at `offset` of the file `name` in the host's
// output directory, creating it at offset 0, and returns how many, or a
// negated errno.
static inline long rvsim_write_file(const void *buf, unsigned long len,
                                    unsigned long offset, const char *name) {
  register unsigned long rvsim_a0_ __asm__("a0") = (unsigned long)buf;
  register unsigned long rvsim_a1_ __asm__("a1") = len;
  register unsigned long rvsim_a2_ __asm__("a2") = offset;
  register unsigned long rvsim_a3_ __asm__("a3") = (unsigned long)name;
  __asm__ volatile(".word 0x7b | (" RVSIM_STR(RVSIM_OP_WRITE_FILE) " << 25)"
                   : "+r"(rvsim_a0_)
                   : "r"(rvsim_a1_), "r"(rvsim_a2_), "r"(rvsim_a3_)
                   : "memory");
  return (long)rvsim_a0_;
}

#endif
