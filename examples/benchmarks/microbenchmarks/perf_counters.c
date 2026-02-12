#include "bench.h"
#include "stdio.h"

int main() {
  unsigned long start, end;

  // Workload
  volatile int k = 0;

  start = read_cycles();
  for (int i = 0; i < 10000; i++) {
    k += i;
  }
  end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
