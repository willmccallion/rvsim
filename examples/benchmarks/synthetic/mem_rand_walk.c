#include "bench.h"
#include "stdio.h"

#define SIZE 16384
struct Node {
  struct Node *next;
  long pad[7];
}; // 64 bytes
struct Node pool[SIZE];

int main() {
  // Setup random links (Excluded from timing)
  for (int i = 0; i < SIZE - 1; i++)
    pool[i].next = &pool[(i * 1237) % SIZE];
  pool[SIZE - 1].next = &pool[0];

  struct Node *curr = &pool[0];

  unsigned long start = read_cycles();
  for (int i = 0; i < 100000; i++)
    curr = curr->next;
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  return 0;
}
