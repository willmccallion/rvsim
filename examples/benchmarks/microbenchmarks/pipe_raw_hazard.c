#include "bench.h"
#include "stdio.h"

int main() {
  volatile int a = 1, b = 2, c = 3;

  unsigned long start = read_cycles();
  for (int i = 0; i < 5000; i++) {
    a = b + c; // Produces a
    c = a + b; // Consumes a immediately
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
