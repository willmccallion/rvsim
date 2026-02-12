#include "bench.h"
#include "stdio.h"

#define N 1024
int arr[N];

int main() {
  // Initialize with reverse data to force worst-case bubble sort (Excluded)
  for (int i = 0; i < N; i++)
    arr[i] = N - i;

  unsigned long start = read_cycles();
  // Bubble sort
  for (int i = 0; i < N - 1; i++)
    for (int j = 0; j < N - i - 1; j++)
      if (arr[j] > arr[j + 1]) {
        int t = arr[j];
        arr[j] = arr[j + 1];
        arr[j + 1] = t;
      }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
