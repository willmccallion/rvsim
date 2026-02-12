#include "bench.h"
#include "stdio.h"
#include "stdlib.h"

// 64x64 Image
#define W 64
#define H 64

unsigned char img[H][W];
unsigned char out[H][W];

int abs_int(int x) { return x < 0 ? -x : x; }

// Initializes a gradient pattern
void init_image() {
  for (int y = 0; y < H; y++) {
    for (int x = 0; x < W; x++) {
      // Create a checkerboard + gradient
      int val = (x * 4) ^ (y * 4);
      img[y][x] = (unsigned char)(val & 0xFF);
    }
  }
}

void sobel_filter() {
  // Sobel Kernels
  // Gx: -1 0 1
  //     -2 0 2
  //     -1 0 1
  // Gy: -1 -2 -1
  //      0  0  0
  //      1  2  1

  for (int y = 1; y < H - 1; y++) {
    for (int x = 1; x < W - 1; x++) {
      int gx = -1 * img[y - 1][x - 1] + 1 * img[y - 1][x + 1] +
               -2 * img[y][x - 1] + 2 * img[y][x + 1] + -1 * img[y + 1][x - 1] +
               1 * img[y + 1][x + 1];

      int gy = -1 * img[y - 1][x - 1] - 2 * img[y - 1][x] -
               1 * img[y - 1][x + 1] + 1 * img[y + 1][x - 1] +
               2 * img[y + 1][x] + 1 * img[y + 1][x + 1];

      // Approximation of magnitude: |Gx| + |Gy|
      int mag = abs_int(gx) + abs_int(gy);

      // Clamp
      if (mag > 255)
        mag = 255;
      out[y][x] = (unsigned char)mag;
    }
  }
}

int main() {
  printf("Sobel Edge Detection (64x64)\n");
  init_image();

  unsigned long start = read_cycles();
  // Run multiple passes to make the benchmark last longer
  for (int i = 0; i < 10; i++) {
    sobel_filter();
  }
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);

  // ASCII Art Visualization of the result (center patch)
  printf("Output Center Patch:\n");
  char ramp[] = " .:-=+*#%@";
  for (int y = 30; y < 38; y++) {
    for (int x = 30; x < 38; x++) {
      int val = out[y][x];
      printf("%c", ramp[val / 26]);
    }
    printf("\n");
  }

  return 0;
}
