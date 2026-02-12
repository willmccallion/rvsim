#include "bench.h"
#include "stdio.h"

#define SIZE 4096
long data[SIZE];

int main() {
  unsigned long start = read_cycles();
  for (int k = 0; k < 100; k++)
    for (int i = 0; i < SIZE; i++)
      data[i] += 1;
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
