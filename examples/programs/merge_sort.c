#include "stdio.h"
#include "stdlib.h"

#define SIZE 102400

long arr[SIZE];

static unsigned long long seed = 999;
long rand_next(void) {
  seed = seed * 6364136223846793005ULL + 1;
  return (long)(seed >> 33);
}

void merge(long arr[], int l, int m, int r) {
  int i, j, k;
  int n1 = m - l + 1;
  int n2 = r - m;

  // Allocate temps
  long *L = (long *)malloc(n1 * sizeof(long));
  long *R = (long *)malloc(n2 * sizeof(long));

  if (!L || !R) {
    printf("FATAL: Malloc failed at depth %d (n1=%d, n2=%d)\n", l, n1, n2);
    // Spin forever to catch the error
    while (1)
      ;
  }

  // Copy data
  for (i = 0; i < n1; i++)
    L[i] = arr[l + i];
  for (j = 0; j < n2; j++)
    R[j] = arr[m + 1 + j];

  // Merge
  i = 0;
  j = 0;
  k = l;
  while (i < n1 && j < n2) {
    if (L[i] <= R[j]) {
      arr[k] = L[i];
      i++;
    } else {
      arr[k] = R[j];
      j++;
    }
    k++;
  }

  while (i < n1) {
    arr[k] = L[i];
    i++;
    k++;
  }
  while (j < n2) {
    arr[k] = R[j];
    j++;
    k++;
  }

  free(L);
  free(R);
}

void merge_sort(long arr[], int l, int r) {
  if (l < r) {
    int m = l + (r - l) / 2;
    merge_sort(arr, l, m);
    merge_sort(arr, m + 1, r);
    merge(arr, l, m, r);
  }
}

int main(void) {
  printf("Initializing array with %d elements...\n", SIZE);
  for (int i = 0; i < SIZE; i++)
    arr[i] = rand_next() % 1000;

  printf("Starting Merge Sort...\n");
  merge_sort(arr, 0, SIZE - 1);

  printf("Verifying...\n");
  int sorted = 1;
  for (int i = 0; i < SIZE - 1; i++) {
    if (arr[i] > arr[i + 1]) {
      printf("Error at index %d: %d > %d\n", i, arr[i], arr[i + 1]);
      sorted = 0;
      break;
    }
  }

  // #region agent log
  // Write debug info to memory-mapped debug region at 0x80010000 (1MB into RAM)
  volatile int *debug_sorted = (volatile int *)0x80010000;
  volatile int *debug_marker = (volatile int *)0x80010004;
  *debug_sorted = sorted;
  *debug_marker = 0xDEADBEEF; // Marker for "verification complete"
  // #endregion

  if (sorted) {
    // #region agent log
    *debug_marker = 0x50000001; // Marker for "about to print SUCCESS"
    // #endregion
    printf("SUCCESS: Array is sorted.\n");
    // #region agent log
    *debug_marker = 0x50000002; // Marker for "after SUCCESS printf"
    // #endregion
  } else {
    // #region agent log
    *debug_marker = 0x60000001; // Marker for "about to print FAILURE"
    // #endregion
    printf("FAILURE: Array is NOT sorted.\n");
  }

  // #region agent log
  *debug_marker = 0x70000000; // Marker for "about to return from main"
  // #endregion
  return 0;
}
