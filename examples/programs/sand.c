#include "stdio.h"
#include "stdlib.h"

// Dimensions must be powers of 2 for shift optimization
// 32 = 1 << 5
// 64 = 1 << 6
#define ROWS 32
#define COLS 64
#define FRAMES 1000

// ANSI Colors
#define CLEAR_SCREEN "\x1b[2J"
#define MOVE_HOME "\x1b[H"
#define COLOR_SAND "\x1b[33m" // Yellow
#define COLOR_WALL "\x1b[37m" // White
#define COLOR_RESET "\x1b[0m"

// Cell Types
#define EMPTY 0
#define WALL 1
#define SAND 2

static unsigned long long seed = 9999;

long rand_next(void) {
  if (seed == 0)
    seed = 123456789;
  seed ^= seed << 13;
  seed ^= seed >> 7;
  seed ^= seed << 17;
  return (long)(seed & 0x7FFFFFFFFFFFFFFF);
}

void sleep_busy(int cycles) {
  for (volatile int i = 0; i < cycles; i++)
    ;
}

int get_idx(int r, int c) { return (r << 6) + c; }

int main() {
  printf("Allocating Physics Grid...\n");
  char *grid = (char *)malloc(ROWS * COLS);

  if (!grid) {
    printf("Malloc failed.\n");
    return 1;
  }

  for (int r = 0; r < ROWS; r++) {
    for (int c = 0; c < COLS; c++) {
      int idx = get_idx(r, c);
      if (r == ROWS - 1 || c == 0 || c == COLS - 1) {
        grid[idx] = WALL;
      } else {
        grid[idx] = EMPTY;
      }
    }
  }

  for (int c = 10; c < 30; c++)
    grid[get_idx(20, c)] = WALL;

  for (int c = 34; c < 54; c++)
    grid[get_idx(12, c)] = WALL;

  int spout_x = 10;
  int spout_dir = 1;

  for (int frame = 0; frame < FRAMES; frame++) {
    printf(CLEAR_SCREEN MOVE_HOME);
    printf("Falling Sand - Frame %d\n", frame);

    for (int r = 0; r < ROWS; r++) {
      for (int c = 0; c < COLS; c++) {
        char cell = grid[get_idx(r, c)];
        if (cell == WALL)
          printf(COLOR_WALL "#" COLOR_RESET);
        else if (cell == SAND)
          printf(COLOR_SAND "." COLOR_RESET);
        else
          printf(" ");
      }
      printf("\n");
    }

    spout_x += spout_dir;
    if (spout_x > 50)
      spout_dir = -1;
    if (spout_x < 10)
      spout_dir = 1;

    if (grid[get_idx(1, spout_x)] == EMPTY) {
      grid[get_idx(1, spout_x)] = SAND;
    }

    for (int r = ROWS - 2; r >= 0; r--) {
      int start_c = 1;
      int end_c = COLS - 1;
      int step = 1;

      if (rand_next() & 1) {
        start_c = COLS - 2;
        end_c = 0;
        step = -1;
      }

      for (int c = start_c; c != end_c; c += step) {
        int idx = get_idx(r, c);

        if (grid[idx] == SAND) {
          int below = get_idx(r + 1, c);
          int below_left = get_idx(r + 1, c - 1);
          int below_right = get_idx(r + 1, c + 1);

          if (grid[below] == EMPTY) {
            grid[below] = SAND;
            grid[idx] = EMPTY;
          } else if (grid[below_left] == EMPTY) {
            grid[below_left] = SAND;
            grid[idx] = EMPTY;
          } else if (grid[below_right] == EMPTY) {
            grid[below_right] = SAND;
            grid[idx] = EMPTY;
          }
        }
      }
    }

    sleep_busy(50000);
  }

  free(grid);
  return 0;
}
