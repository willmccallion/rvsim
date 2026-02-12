#include "bench.h"
#include "stdio.h"

long val = 10;

int main() {
  volatile long res = 0;

  unsigned long start = read_cycles();
  for (int i = 0; i < 5000; i++) {
    // Load val, immediately use it. Should stall 1 cycle.
    res += val;
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
