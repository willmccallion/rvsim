#include "stdio.h"

#define WIDTH 60
#define HEIGHT 30
#define MAX_ITER 32

#define SHIFT 12
#define ONE (1 << SHIFT)

#define TO_FIX(x) ((x) << SHIFT)
#define TO_INT(x) ((x) >> SHIFT)

long soft_mul(long a, long b) {
  long result = 0;
  int negative = 0;

  // Handle signs
  if (a < 0) {
    a = -a;
    negative = !negative;
  }
  if (b < 0) {
    b = -b;
    negative = !negative;
  }

  // Shift and Add
  while (b > 0) {
    if (b & 1) {
      result += a;
    }
    a <<= 1;
    b >>= 1;
  }

  return negative ? -result : result;
}

long fix_mul(long a, long b) {
  long res = soft_mul(a, b);
  return res >> SHIFT;
}

int main() {
  printf("Rendering Mandelbrot Set (Software Multiply)...\n");
  printf("Resolution: %dx%d\n", WIDTH, HEIGHT);

  // Viewport: x from -2.0 to 1.0, y from -1.0 to 1.0
  long x_min = TO_FIX(-2);
  long x_max = TO_FIX(1);
  long y_min = TO_FIX(-1);
  long y_max = TO_FIX(1);

  long dx = (x_max - x_min) / WIDTH;
  long dy = (y_max - y_min) / HEIGHT;

  char chars[] = " .:-=+*#%@";

  for (int y_pix = 0; y_pix < HEIGHT; y_pix++) {
    long cy = y_min + soft_mul(y_pix, dy);

    for (int x_pix = 0; x_pix < WIDTH; x_pix++) {
      long cx = x_min + soft_mul(x_pix, dx);

      long zx = 0;
      long zy = 0;
      int iter = 0;

      // Z = Z^2 + C
      while (iter < MAX_ITER) {
        long zx2 = fix_mul(zx, zx);
        long zy2 = fix_mul(zy, zy);

        if ((zx2 + zy2) > TO_FIX(4)) {
          break;
        }

        long two_zx_zy = fix_mul(zx, zy) << 1;

        long zx_new = zx2 - zy2 + cx;
        zy = two_zx_zy + cy;
        zx = zx_new;

        iter++;
      }

      if (iter == MAX_ITER) {
        printf(" ");
      } else {
        int char_idx = iter & 7;
        printf("%c", chars[char_idx]);
      }
    }
    printf("\n");
  }

  printf("Done.\n");
  return 0;
}
