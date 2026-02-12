#include "bench.h"
#include "stdio.h"

int main() {
  volatile int sum = 0;

  unsigned long start = read_cycles();
  for (int i = 0; i < 10000; i++) {
    if (i % 2 == 0)
      sum++; // Taken, Not Taken...
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
