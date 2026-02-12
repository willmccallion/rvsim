#include "stdio.h"
#include "stdlib.h"

#define SIZE 4

// ANSI Colors
#define CLEAR_SCREEN "\x1b[2J"
#define MOVE_HOME "\x1b[H"
#define COLOR_RESET "\x1b[0m"
#define COLOR_BORDER "\x1b[37m" // White

// Number Colors
#define C_2 "\x1b[37m"      // White
#define C_4 "\x1b[36m"      // Cyan
#define C_8 "\x1b[32m"      // Green
#define C_16 "\x1b[33m"     // Yellow
#define C_32 "\x1b[31m"     // Red
#define C_64 "\x1b[35m"     // Magenta
#define C_128 "\x1b[1;34m"  // Bold Blue
#define C_HIGH "\x1b[1;31m" // Bold Red

int board[SIZE][SIZE];
int score = 0;
int win = 0;

// Random Number Generator
static unsigned long long seed = 8888;
long rand_next(void) {
  seed = seed * 6364136223846793005ULL + 1;
  return (long)(seed >> 33);
}

void spawn_tile() {
  int empty[SIZE * SIZE][2];
  int count = 0;

  for (int r = 0; r < SIZE; r++) {
    for (int c = 0; c < SIZE; c++) {
      if (board[r][c] == 0) {
        empty[count][0] = r;
        empty[count][1] = c;
        count++;
      }
    }
  }

  if (count > 0) {
    int idx = rand_next() % count;
    int r = empty[idx][0];
    int c = empty[idx][1];
    // 90% chance of 2, 10% chance of 4
    board[r][c] = ((rand_next() % 10) == 0) ? 4 : 2;
  }
}

void init_game() {
  score = 0;
  win = 0;
  for (int i = 0; i < SIZE * SIZE; i++) {
    board[i / SIZE][i % SIZE] = 0;
  }
  spawn_tile();
  spawn_tile();
}

const char *get_color(int val) {
  if (val == 0)
    return COLOR_RESET;
  if (val == 2)
    return C_2;
  if (val == 4)
    return C_4;
  if (val == 8)
    return C_8;
  if (val == 16)
    return C_16;
  if (val == 32)
    return C_32;
  if (val == 64)
    return C_64;
  if (val == 128)
    return C_128;
  return C_HIGH;
}

void draw() {
  printf(CLEAR_SCREEN MOVE_HOME);
  printf("2048 - Score: %d\n\n", score);

  printf(COLOR_BORDER "+------+------+------+------+\n");

  for (int r = 0; r < SIZE; r++) {
    printf("|");
    for (int c = 0; c < SIZE; c++) {
      int val = board[r][c];
      printf(get_color(val));

      if (val == 0)
        printf("      ");
      else if (val < 10)
        printf("   %d  ", val);
      else if (val < 100)
        printf("  %d  ", val);
      else if (val < 1000)
        printf("  %d ", val);
      else
        printf(" %d ", val);

      printf(COLOR_BORDER "|");
    }
    printf("\n+------+------+------+------+\n" COLOR_RESET);
  }

  printf("\nControls: w, a, s, d (then Enter)\n");
  printf("q to quit, r to restart\n");
  printf("> ");
}

// Logic to slide/merge a single row to the left
int slide_row(int *row) {
  int moved = 0;
  // Shift non-zeros to left
  int temp[SIZE] = {0};
  int t_idx = 0;
  for (int i = 0; i < SIZE; i++) {
    if (row[i] != 0)
      temp[t_idx++] = row[i];
  }

  // Merge
  for (int i = 0; i < SIZE - 1; i++) {
    if (temp[i] != 0 && temp[i] == temp[i + 1]) {
      temp[i] *= 2;
      score += temp[i];
      if (temp[i] == 2048)
        win = 1;
      temp[i + 1] = 0;
      // Shift everything after this left
      for (int j = i + 1; j < SIZE - 1; j++) {
        temp[j] = temp[j + 1];
      }
      temp[SIZE - 1] = 0;
      moved = 1; // A merge counts as a move
    }
  }

  // Copy back and check if changed
  for (int i = 0; i < SIZE; i++) {
    if (row[i] != temp[i]) {
      moved = 1;
      row[i] = temp[i];
    }
  }
  return moved;
}

void rotate_board() {
  int temp[SIZE][SIZE];
  for (int r = 0; r < SIZE; r++) {
    for (int c = 0; c < SIZE; c++) {
      temp[c][SIZE - 1 - r] = board[r][c];
    }
  }
  for (int i = 0; i < SIZE * SIZE; i++)
    board[i / SIZE][i % SIZE] = temp[i / SIZE][i % SIZE];
}

int move_left() {
  int moved = 0;
  for (int r = 0; r < SIZE; r++) {
    if (slide_row(board[r]))
      moved = 1;
  }
  return moved;
}

int move_right() {
  rotate_board();
  rotate_board();
  int moved = move_left();
  rotate_board();
  rotate_board();
  return moved;
}

int move_up() {
  rotate_board();
  rotate_board();
  rotate_board();
  int moved = move_left();
  rotate_board();
  return moved;
}

int move_down() {
  rotate_board();
  int moved = move_left();
  rotate_board();
  rotate_board();
  rotate_board();
  return moved;
}

int main() {
  init_game();
  char buf[16];
  int needs_redraw = 1; // Draw once at start

  while (1) {
    if (needs_redraw) {
      draw();
      if (win) {
        printf(C_128 "\nYOU WIN! (2048 Reached)\n" COLOR_RESET);
        win = 0; // Continue playing
      }
      needs_redraw = 0;
    }

    // Use gets to wait for Enter key
    gets(buf, 16);
    char c = buf[0];

    int moved = 0;
    if (c == 'w')
      moved = move_up();
    else if (c == 'a')
      moved = move_left();
    else if (c == 's')
      moved = move_down();
    else if (c == 'd')
      moved = move_right();
    else if (c == 'q')
      break;
    else if (c == 'r') {
      init_game();
      needs_redraw = 1;
      continue;
    }

    if (moved) {
      spawn_tile();
      needs_redraw = 1;
    }
  }

  printf("Thanks for playing!\n");
  return 0;
}
