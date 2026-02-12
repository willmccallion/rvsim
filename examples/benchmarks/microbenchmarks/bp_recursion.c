#include "bench.h"
#include "stdio.h"

int recurse(int n) {
  if (n <= 0)
    return 0;
  return 1 + recurse(n - 1);
}

int main() {
  unsigned long start = read_cycles();
  recurse(1000);
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
