// Harts 0 and 1 hand a token back and forth; reports the round-trip cost.
#include "smp.h"
#include "stdio.h"

#define ROUNDS 1000

static volatile uint64_t token;
static smp_barrier_t barrier;

int smp_main(unsigned long hart, unsigned long harts) {
  smp_barrier(&barrier, harts);
  if (hart > 1) {
    return 0;
  }
  unsigned long start = smp_cycles();
  for (uint64_t round = 0; round < ROUNDS; round++) {
    uint64_t mine = 2 * round + hart;
    while (token != mine) {
    }
    token = mine + 1;
  }
  unsigned long elapsed = smp_cycles() - start;
  if (hart != 0) {
    return 0;
  }
  while (token != 2 * ROUNDS) {
  }
  printf("pingpong: %d round trips, %lu cycles, %lu cycles per hand-off\n", ROUNDS, elapsed,
         elapsed / (2 * ROUNDS));
  return 0;
}
