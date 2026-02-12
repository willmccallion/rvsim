#include "bench.h"
#include "stdio.h"

int main() {
  volatile int sum = 0;

  unsigned long start = read_cycles();
  for (int i = 0; i < 10000; i++) {
    switch (i % 4) {
    case 0:
      sum += 1;
      break;
    case 1:
      sum += 2;
      break;
    case 2:
      sum += 3;
      break;
    case 3:
      sum += 4;
      break;
    }
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
