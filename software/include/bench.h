#ifndef BENCH_H
#define BENCH_H

// Reads the cycle counter (CSR 0xC00)
static inline unsigned long read_cycles(void) {
  unsigned long cycles;
  asm volatile("rdcycle %0" : "=r"(cycles));
  return cycles;
}

// Reads the instructions retired counter (CSR 0xC02)
static inline unsigned long read_instret(void) {
  unsigned long insts;
  asm volatile("rdinstret %0" : "=r"(insts));
  return insts;
}

#endif
