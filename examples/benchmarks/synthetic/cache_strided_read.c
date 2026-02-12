#include "bench.h"
#include "stdio.h"

#define SIZE (1024 * 64)
#define STRIDE 16
long data[SIZE];

int main() {
  // Initialization (Excluded)
  for (int i = 0; i < SIZE; i++)
    data[i] = i;

  volatile long sum = 0;

  unsigned long start = read_cycles();
  for (int i = 0; i < SIZE; i += STRIDE)
    sum += data[i];
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
