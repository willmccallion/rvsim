#include <stdio.h>
#include <stdlib.h>

#define WIDTH 60
#define HEIGHT 30

int main() {
  char buf[16];
  int max_iter = 32;

  printf("Mandelbrot Set (Floating Point)\n");
  printf("Enter Max Iterations (default 32): ");

  if (gets(buf, sizeof(buf))) {
    if (buf[0] != '\n' && buf[0] != 0) {
      max_iter = atoi(buf);
    }
  }

  if (max_iter <= 0)
    max_iter = 32;

  printf("Rendering with %d iterations...\n", max_iter);

  double x_min = -2.0;
  double x_max = 1.0;
  double y_min = -1.0;
  double y_max = 1.0;

  double dx = (x_max - x_min) / WIDTH;
  double dy = (y_max - y_min) / HEIGHT;

  char chars[] = " .:-=+*#%@";

  for (int y_pix = 0; y_pix < HEIGHT; y_pix++) {
    double cy = y_min + (y_pix * dy);

    for (int x_pix = 0; x_pix < WIDTH; x_pix++) {
      double cx = x_min + (x_pix * dx);

      double zx = 0.0;
      double zy = 0.0;
      int iter = 0;

      while (iter < max_iter) {
        double zx2 = zx * zx;
        double zy2 = zy * zy;

        // Escape condition > 4.0
        if ((zx2 + zy2) > 4.0)
          break;

        double two_zx_zy = 2.0 * zx * zy;

        zx = zx2 - zy2 + cx;
        zy = two_zx_zy + cy;
        iter++;
      }

      if (iter == max_iter) {
        printf(" ");
      } else {
        int char_idx = iter % 10;
        printf("%c", chars[char_idx]);
      }
    }
    printf("\n");
  }

  printf("Done.\n");
  return 0;
}
