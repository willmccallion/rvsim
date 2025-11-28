#include "stdio.h"
#include "stdlib.h"

// Use Powers of 2 for dimensions to avoid MUL instruction
// 32 = 1 << 5
// 64 = 1 << 6
#define ROWS 32
#define COLS 64
#define GENERATIONS 500

// ANSI Escape Codes
#define CLEAR_SCREEN "\x1b[2J"
#define MOVE_HOME "\x1b[H"
#define COLOR_ALIVE "\x1b[32m" // Green
#define COLOR_RESET "\x1b[0m"

static unsigned long long seed = 8888;

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

int main() {
  printf("Conway's Game of Life (Safe Mode)\n");

  char *grid = (char *)malloc(ROWS * COLS);
  char *next = (char *)malloc(ROWS * COLS);

  if (!grid || !next) {
    printf("FATAL: Malloc failed!\n");
    return 1;
  }

  printf("Initializing grid...\n");
  int alive_count = 0;
  for (int i = 0; i < ROWS * COLS; i++) {
    if ((rand_next() & 0x7F) < 32) {
      grid[i] = 1;
      alive_count++;
    } else {
      grid[i] = 0;
    }
  }
  printf("Grid initialized. Alive cells: %d\n", alive_count);
  sleep_busy(1000000);

  for (int gen = 0; gen < GENERATIONS; gen++) {
    printf(CLEAR_SCREEN MOVE_HOME);
    printf("Generation: %d\n", gen);

    printf("+");
    for (int c = 0; c < COLS; c++)
      printf("-");
    printf("+\n");

    for (int r = 0; r < ROWS; r++) {
      printf("|");
      for (int c = 0; c < COLS; c++) {
        int idx = (r << 6) + c;

        if (grid[idx]) {
          printf(COLOR_ALIVE "O" COLOR_RESET);
        } else {
          printf(" ");
        }
      }
      printf("|\n");
    }

    printf("+");
    for (int c = 0; c < COLS; c++)
      printf("-");
    printf("+\n");

    for (int r = 0; r < ROWS; r++) {
      for (int c = 0; c < COLS; c++) {
        int neighbors = 0;

        for (int dr = -1; dr <= 1; dr++) {
          for (int dc = -1; dc <= 1; dc++) {
            if (dr == 0 && dc == 0)
              continue;

            int nr = r + dr;
            int nc = c + dc;

            nr = nr & (ROWS - 1);
            nc = nc & (COLS - 1);

            int n_idx = (nr << 6) + nc;

            if (grid[n_idx])
              neighbors++;
          }
        }

        int idx = (r << 6) + c;
        int alive = grid[idx];

        if (alive && (neighbors < 2 || neighbors > 3)) {
          next[idx] = 0;
        } else if (!alive && neighbors == 3) {
          next[idx] = 1;
        } else {
          next[idx] = alive;
        }
      }
    }

    char *temp = grid;
    grid = next;
    next = temp;

    sleep_busy(100000);
  }

  free(grid);
  free(next);
  return 0;
}
