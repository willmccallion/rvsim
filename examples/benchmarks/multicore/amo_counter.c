// Every hart adds to one shared word with amoadd; the total must be exact.
#include "smp.h"
#include "stdio.h"

#define ITERATIONS 4000

static volatile uint64_t counter;
static smp_barrier_t barrier;

int smp_main(unsigned long hart, unsigned long harts) {
  for (int i = 0; i < ITERATIONS; i++) {
    smp_atomic_add(&counter, 1);
  }
  smp_barrier(&barrier, harts);
  if (hart != 0) {
    return 0;
  }
  uint64_t expected = (uint64_t)harts * ITERATIONS;
  printf("amo_counter: %lu harts, counter=%lu expected=%lu\n", harts, counter, expected);
  return counter == expected ? 0 : 1;
}
