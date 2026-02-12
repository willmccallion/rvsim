#include "bench.h"
#include "stdio.h"

int main() {
  volatile double a = 1.0;

  unsigned long start = read_cycles();
  for (int i = 0; i < 10000; i++) {
    // a = a * b + c
    asm volatile("fmadd.d %0, %1, %2, %0" : "+f"(a) : "f"(1.001), "f"(0.5));
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
