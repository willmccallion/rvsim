#include "bench.h"
#include "stdio.h"

int main() {
  volatile double a = 1.0;

  unsigned long start = read_cycles();
  for (int i = 0; i < 10000; i++)
    a = a + 1.0001;
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
