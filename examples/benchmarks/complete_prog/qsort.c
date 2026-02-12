#include "bench.h"
#include "stdio.h"

#define SIZE 32768

long arr[SIZE];

static unsigned long long seed = 123456789;

long rand_next(void) {
  seed = seed * 6364136223846793005ULL + 1;
  return (long)(seed >> 33); // Return top 31 bits
}

void swap(long *a, long *b) {
  long t = *a;
  *a = *b;
  *b = t;
}

// Partition the array using the last element as the pivot
int partition(long array[], int low, int high) {
  long pivot = array[high];
  int i = (low - 1);

  for (int j = low; j < high; j++) {
    if (array[j] <= pivot) {
      i++;
      swap(&array[i], &array[j]);
    }
  }
  swap(&array[i + 1], &array[high]);
  return (i + 1);
}

void quick_sort(long array[], int low, int high) {
  if (low < high) {
    int pi = partition(array, low, high);

    // Recursive calls
    quick_sort(array, low, pi - 1);
    quick_sort(array, pi + 1, high);
  }
}

int verify_sorted(long array[], int size) {
  for (int i = 0; i < size - 1; i++) {
    if (array[i] > array[i + 1]) {
      printf("Error at index %d: %d > %d\n", i, array[i], array[i + 1]);
      return 0;
    }
  }
  return 1;
}

int main(void) {
  printf("Initializing array with %d random elements...\n", SIZE);

  for (int i = 0; i < SIZE; i++) {
    arr[i] = rand_next() % 10000; // Random numbers 0-9999
  }

  printf("Starting Quick Sort...\n");

  unsigned long start = read_cycles();
  quick_sort(arr, 0, SIZE - 1);
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);

  if (verify_sorted(arr, SIZE)) {
    printf("SUCCESS: Array is sorted.\n");
  } else {
    printf("FAILURE: Array is NOT sorted.\n");
  }

  return 0;
}
