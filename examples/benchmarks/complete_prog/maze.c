#include "bench.h"
#include "stdio.h"
#include "stdlib.h"

#define COLS 32
#define ROWS 16

#define VISITED 0x01
#define WALL_N 0x02
#define WALL_S 0x04
#define WALL_E 0x08
#define WALL_W 0x10

#define PATH_MARKER 0x20

#define CLEAR_SCREEN "\x1b[2J"
#define MOVE_HOME "\x1b[H"

#define COLOR_WALL "\x1b[40m"
#define COLOR_EMPTY "\x1b[47m"
#define COLOR_PATH "\x1b[41;30m"
#define COLOR_RESET "\x1b[0m"

static unsigned long long seed = 9999;

long rand_next(void) {
  if (seed == 0)
    seed = 123456789;
  seed ^= seed << 13;
  seed ^= seed >> 7;
  seed ^= seed << 17;
  return (long)(seed & 0x7FFFFFFFFFFFFFFF);
}

int idx(int r, int c) { return (r << 5) + c; }

int abs(int x) { return (x < 0) ? -x : x; }

int heuristic(int r, int c, int goal_r, int goal_c) {
  return abs(r - goal_r) + abs(c - goal_c);
}

typedef struct {
  int r;
  int c;
} Point;

void generate_maze(unsigned char *grid) {
  Point *stack = (Point *)malloc(sizeof(Point) * ROWS * COLS);
  int sp = 0;

  for (int i = 0; i < ROWS * COLS; i++)
    grid[i] = WALL_N | WALL_S | WALL_E | WALL_W;

  int curr_r = 0, curr_c = 0;
  grid[idx(0, 0)] |= VISITED;
  stack[sp++] = (Point){0, 0};

  int visited_count = 1;
  int total = ROWS * COLS;

  while (visited_count < total && sp > 0) {
    curr_r = stack[sp - 1].r;
    curr_c = stack[sp - 1].c;

    int neighbors[4];
    int n_count = 0;

    if (curr_r > 0 && !(grid[idx(curr_r - 1, curr_c)] & VISITED))
      neighbors[n_count++] = 0;
    if (curr_r < ROWS - 1 && !(grid[idx(curr_r + 1, curr_c)] & VISITED))
      neighbors[n_count++] = 1;
    if (curr_c < COLS - 1 && !(grid[idx(curr_r, curr_c + 1)] & VISITED))
      neighbors[n_count++] = 2;
    if (curr_c > 0 && !(grid[idx(curr_r, curr_c - 1)] & VISITED))
      neighbors[n_count++] = 3;

    if (n_count > 0) {
      int r = rand_next() & 0xF;
      while (r >= n_count)
        r -= n_count;
      int dir = neighbors[r];

      int next_r = curr_r, next_c = curr_c;

      if (dir == 0) {
        grid[idx(curr_r, curr_c)] &= ~WALL_N;
        next_r--;
        grid[idx(next_r, next_c)] &= ~WALL_S;
      } else if (dir == 1) {
        grid[idx(curr_r, curr_c)] &= ~WALL_S;
        next_r++;
        grid[idx(next_r, next_c)] &= ~WALL_N;
      } else if (dir == 2) {
        grid[idx(curr_r, curr_c)] &= ~WALL_E;
        next_c++;
        grid[idx(next_r, next_c)] &= ~WALL_W;
      } else if (dir == 3) {
        grid[idx(curr_r, curr_c)] &= ~WALL_W;
        next_c--;
        grid[idx(next_r, next_c)] &= ~WALL_E;
      }

      grid[idx(next_r, next_c)] |= VISITED;
      stack[sp++] = (Point){next_r, next_c};
      visited_count++;
    } else {
      sp--;
    }
  }
  free(stack);
}

// A* Solver
void solve_astar(unsigned char *grid) {
  int total_nodes = ROWS * COLS;

  int *g_score = (int *)malloc(total_nodes * sizeof(int));
  int *f_score = (int *)malloc(total_nodes * sizeof(int));
  int *parent = (int *)malloc(total_nodes * sizeof(int));
  int *in_open = (int *)malloc(total_nodes * sizeof(int));

  for (int i = 0; i < total_nodes; i++) {
    g_score[i] = 999999;
    f_score[i] = 999999;
    parent[i] = -1;
    in_open[i] = 0;
  }

  int start_idx = idx(0, 0);
  int goal_idx = idx(ROWS - 1, COLS - 1);

  g_score[start_idx] = 0;
  f_score[start_idx] = heuristic(0, 0, ROWS - 1, COLS - 1);
  in_open[start_idx] = 1;

  printf("Running A* Solver...\n");

  while (1) {
    int current = -1;
    int lowest_f = 999999;

    for (int i = 0; i < total_nodes; i++) {
      if (in_open[i] && f_score[i] < lowest_f) {
        lowest_f = f_score[i];
        current = i;
      }
    }

    if (current == -1) {
      printf("No path found!\n");
      break;
    }

    if (current == goal_idx) {
      printf("Goal Reached! Reconstructing path...\n");
      int trace = goal_idx;
      while (trace != -1) {
        grid[trace] |= PATH_MARKER;
        trace = parent[trace];
      }
      break;
    }

    in_open[current] = 0;

    int r = current >> 5;
    int c = current & 31;

    int dr[] = {-1, 1, 0, 0};
    int dc[] = {0, 0, 1, -1};
    int walls[] = {WALL_N, WALL_S, WALL_E, WALL_W};

    for (int i = 0; i < 4; i++) {
      int nr = r + dr[i];
      int nc = c + dc[i];

      if (nr >= 0 && nr < ROWS && nc >= 0 && nc < COLS) {
        if (!(grid[current] & walls[i])) {
          int neighbor_idx = idx(nr, nc);
          int tentative_g = g_score[current] + 1;

          if (tentative_g < g_score[neighbor_idx]) {
            parent[neighbor_idx] = current;
            g_score[neighbor_idx] = tentative_g;
            f_score[neighbor_idx] =
                tentative_g + heuristic(nr, nc, ROWS - 1, COLS - 1);

            if (!in_open[neighbor_idx]) {
              in_open[neighbor_idx] = 1;
            }
          }
        }
      }
    }
  }

  free(g_score);
  free(f_score);
  free(parent);
  free(in_open);
}

int main() {
  printf("Allocating Grid...\n");
  unsigned char *grid = (unsigned char *)malloc(ROWS * COLS);

  if (!grid) {
    printf("Malloc failed\n");
    return 1;
  }

  printf("Generating Maze...\n");
  generate_maze(grid);

  unsigned long start = read_cycles();
  solve_astar(grid);
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);

  printf(CLEAR_SCREEN MOVE_HOME);
  printf("A* Maze Solver (%dx%d):\n\n", COLS, ROWS);

  for (int c = 0; c < COLS; c++)
    printf(COLOR_WALL "    " COLOR_RESET);
  printf(COLOR_WALL " " COLOR_RESET "\n");

  for (int r = 0; r < ROWS; r++) {
    printf(COLOR_WALL " " COLOR_RESET);

    for (int c = 0; c < COLS; c++) {
      int i = idx(r, c);

      if (grid[i] & PATH_MARKER)
        printf(COLOR_PATH " * " COLOR_RESET);
      else
        printf(COLOR_EMPTY "   " COLOR_RESET);

      if (grid[i] & WALL_E) {
        printf(COLOR_WALL " " COLOR_RESET);
      } else if ((grid[i] & PATH_MARKER) && (c < COLS - 1) &&
                 (grid[idx(r, c + 1)] & PATH_MARKER)) {
        printf(COLOR_PATH " " COLOR_RESET);
      } else {
        printf(COLOR_EMPTY " " COLOR_RESET);
      }
    }
    printf("\n");

    for (int c = 0; c < COLS; c++) {
      int i = idx(r, c);

      if (grid[i] & WALL_S) {
        printf(COLOR_WALL "   " COLOR_RESET);
      } else if ((grid[i] & PATH_MARKER) && (r < ROWS - 1) &&
                 (grid[idx(r + 1, c)] & PATH_MARKER)) {
        printf(COLOR_PATH "   " COLOR_RESET);
      } else {
        printf(COLOR_EMPTY "   " COLOR_RESET);
      }

      printf(COLOR_WALL " " COLOR_RESET);
    }
    printf("\n");
  }

  free(grid);
  return 0;
}
