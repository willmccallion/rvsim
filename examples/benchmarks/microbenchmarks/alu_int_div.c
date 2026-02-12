#include "bench.h"
#include "stdio.h"

int main() {
  volatile long a = 123456789;

  unsigned long start = read_cycles();
  for (int i = 0; i < 10000; i++)
    a = a / 3;
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
