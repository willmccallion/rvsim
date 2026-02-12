#include "bench.h"
#include "stdio.h"

#define N 32
double A[N][N], B[N][N], C[N][N];

int main() {
  // Initialize with dummy data (Excluded)
  for (int i = 0; i < N; i++) {
    for (int j = 0; j < N; j++) {
      A[i][j] = (double)(i + j);
      B[i][j] = (double)(i - j);
    }
  }

  unsigned long start = read_cycles();
  for (int i = 0; i < N; i++)
    for (int j = 0; j < N; j++)
      for (int k = 0; k < N; k++)
        C[i][j] += A[i][k] * B[k][j];
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
