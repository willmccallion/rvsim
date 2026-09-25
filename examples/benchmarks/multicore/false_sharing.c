// Per-hart counters packed into one cache line versus one line each.
#include "smp.h"
#include "stdio.h"

#define ITERATIONS 4000
#define MAX_HARTS 16

static volatile uint64_t packed[MAX_HARTS];
static volatile uint64_t padded[MAX_HARTS * 8];
static smp_barrier_t barrier;
static volatile unsigned long packed_cycles;
static volatile unsigned long padded_cycles;

int smp_main(unsigned long hart, unsigned long harts) {
  smp_barrier(&barrier, harts);
  unsigned long start = smp_cycles();
  for (int i = 0; i < ITERATIONS; i++) {
    packed[hart] = packed[hart] + 1;
  }
  unsigned long packed_elapsed = smp_cycles() - start;
  smp_barrier(&barrier, harts);

  start = smp_cycles();
  for (int i = 0; i < ITERATIONS; i++) {
    padded[hart * 8] = padded[hart * 8] + 1;
  }
  unsigned long padded_elapsed = smp_cycles() - start;
  smp_barrier(&barrier, harts);

  if (hart == 0) {
    packed_cycles = packed_elapsed;
    padded_cycles = padded_elapsed;
  }
  smp_barrier(&barrier, harts);
  if (hart != 0) {
    return 0;
  }
  int ok = 1;
  for (unsigned long h = 0; h < harts; h++) {
    ok &= packed[h] == ITERATIONS && padded[h * 8] == ITERATIONS;
  }
  printf("false_sharing: %lu harts, packed %lu cycles, padded %lu cycles\n", harts, packed_cycles,
         padded_cycles);
  return ok ? 0 : 1;
}
