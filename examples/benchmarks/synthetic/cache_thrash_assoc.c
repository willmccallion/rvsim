#include "bench.h"
#include "stdio.h"

#define WAYS 16
#define SET_STRIDE 4096
long data[WAYS * SET_STRIDE];

int main() {
  // Initialization (Excluded)
  for (int i = 0; i < WAYS * SET_STRIDE; i++)
    data[i] = i;

  volatile long sum = 0;

  unsigned long start = read_cycles();
  for (int iter = 0; iter < 100; iter++) {
    for (int i = 0; i < WAYS; i++) {
      sum += data[i * SET_STRIDE];
    }
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
