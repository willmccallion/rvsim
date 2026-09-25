// A spinlock guards a plain read-modify-write; mutual exclusion must hold.
#include "smp.h"
#include "stdio.h"

#define ITERATIONS 2000

static smp_lock_t lock;
static volatile uint64_t counter;
static smp_barrier_t barrier;

int smp_main(unsigned long hart, unsigned long harts) {
  for (int i = 0; i < ITERATIONS; i++) {
    smp_lock(&lock);
    uint64_t value = counter;
    counter = value + 1;
    smp_unlock(&lock);
  }
  smp_barrier(&barrier, harts);
  if (hart != 0) {
    return 0;
  }
  uint64_t expected = (uint64_t)harts * ITERATIONS;
  printf("spinlock: %lu harts, counter=%lu expected=%lu\n", harts, counter, expected);
  return counter == expected ? 0 : 1;
}
