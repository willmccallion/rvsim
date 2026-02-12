#include "bench.h"
#include "stdio.h"

static unsigned long seed = 123;

int main() {
  volatile int sum = 0;

  unsigned long start = read_cycles();
  for (int i = 0; i < 10000; i++) {
    seed = seed * 1103515245 + 12345;
    if ((seed >> 16) & 1)
      sum++;
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
