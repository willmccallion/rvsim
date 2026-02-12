#include "stdio.h"
#include "stdlib.h"

#define BOARD_SIZE 64
#define EMPTY 0
#define PAWN 1
#define KNIGHT 2
#define BISHOP 3
#define ROOK 4
#define QUEEN 5
#define KING 6

#define WHITE 1
#define BLACK -1
#define INF 30000

// ANSI Colors
#define CLR_RESET "\x1b[0m"
#define CLR_W_PIECE "\x1b[1;36m" // Cyan
#define CLR_B_PIECE "\x1b[1;31m" // Red
#define CLR_MOVE "\x1b[33m"      // Yellow

int board[BOARD_SIZE];
int side = WHITE;
int nodes = 0;

// Piece values
int vals[] = {0, 100, 300, 310, 500, 900, 20000};

// Move Offsets
int n_off[] = {-17, -15, -10, -6, 6, 10, 15, 17};
int k_off[] = {-9, -8, -7, -1, 1, 7, 8, 9};
int b_off[] = {-9, -7, 7, 9};
int r_off[] = {-8, -1, 1, 8};

int abs(int x) { return x < 0 ? -x : x; }

void init_board() {
  int setup[] = {ROOK, KNIGHT, BISHOP, QUEEN, KING, BISHOP, KNIGHT, ROOK};
  for (int i = 0; i < 64; i++)
    board[i] = EMPTY;

  for (int i = 0; i < 8; i++) {
    board[i] = setup[i];       // White Pieces
    board[i + 8] = PAWN;       // White Pawns
    board[i + 48] = -PAWN;     // Black Pawns
    board[i + 56] = -setup[i]; // Black Pieces
  }
  side = WHITE;
}

void print_board() {
  printf("\x1b[H\x1b[2J"); // Clear
  printf("   Chess (8x8)\n\n");

  for (int r = 7; r >= 0; r--) {
    printf(" %d ", r + 1);
    for (int c = 0; c < 8; c++) {
      int idx = r * 8 + c;
      int p = board[idx];

      printf("[");
      if (p == 0)
        printf(" ");
      else {
        printf(p > 0 ? CLR_W_PIECE : CLR_B_PIECE);
        char sym = " PNBRQK"[abs(p)];
        printf("%c", sym);
        printf(CLR_RESET);
      }
      printf("]");
    }
    printf("\n");
  }
  printf("    a  b  c  d  e  f  g  h\n\n");
}

typedef struct {
  int f;
  int t;
  int score;
} Move;

unsigned long rng = 123;
int rand_fast() {
  rng = rng * 1103515245 + 12345;
  return (int)(rng >> 16) & 32767;
}

int evaluate() {
  int score = 0;
  for (int i = 0; i < 64; i++) {
    if (board[i])
      score += (board[i] > 0 ? vals[board[i]] : -vals[-board[i]]);
  }
  return score;
}

int gen_moves(Move *list) {
  int cnt = 0;
  for (int i = 0; i < 64; i++) {
    int p = board[i];
    if (!p || (p > 0) != (side == WHITE))
      continue;

    int type = abs(p);
    int dir = (p > 0) ? 1 : -1;

    if (type == PAWN) {
      int fwd = i + (dir * 8);
      if (fwd >= 0 && fwd < 64 && board[fwd] == EMPTY) {
        list[cnt++] = (Move){i, fwd, 0};

        int row = i / 8;
        if ((dir == 1 && row == 1) || (dir == -1 && row == 6)) {
          int fwd2 = i + (dir * 16);
          if (board[fwd2] == EMPTY) {
            list[cnt++] = (Move){i, fwd2, 0};
          }
        }
      }

      // Capture
      int caps[] = {-1, 1};
      for (int k = 0; k < 2; k++) {
        int cap = fwd + caps[k];
        // Check column wrap for capture
        if (cap >= 0 && cap < 64 && abs((cap % 8) - (i % 8)) == 1) {
          if (board[cap] && (board[cap] > 0) != (p > 0)) {
            list[cnt++] = (Move){i, cap, 0};
          }
        }
      }
    } else {
      // Sliding & Stepping
      int *dirs;
      int len;
      int slide = 0;
      if (type == KNIGHT) {
        dirs = n_off;
        len = 8;
      } else if (type == KING) {
        dirs = k_off;
        len = 8;
      } else {
        slide = 1;
      }

      if (!slide) {
        for (int k = 0; k < len; k++) {
          int dest = i + dirs[k];
          int c_diff = abs((dest % 8) - (i % 8));
          if (dest >= 0 && dest < 64 && c_diff <= 2) {
            if (board[dest] == 0 || (board[dest] > 0) != (p > 0))
              list[cnt++] = (Move){i, dest, 0};
          }
        }
      } else {
        int q_dirs[] = {-9, -8, -7, -1, 1, 7, 8, 9};
        int *s_dirs = (type == BISHOP) ? b_off
                      : (type == ROOK) ? r_off
                                       : q_dirs;
        int s_len = (type == QUEEN) ? 8 : 4;

        for (int k = 0; k < s_len; k++) {
          int curr = i;
          while (1) {
            int next = curr + s_dirs[k];
            if (next < 0 || next >= 64)
              break;
            if (abs((next % 8) - (curr % 8)) > 1)
              break; // Wrapped row

            if (board[next] == 0) {
              list[cnt++] = (Move){i, next, 0};
            } else {
              if ((board[next] > 0) != (p > 0))
                list[cnt++] = (Move){i, next, 0};
              break;
            }
            curr = next;
          }
        }
      }
    }
  }
  return cnt;
}

void apply(Move m) {
  int p = board[m.f];
  board[m.t] = p;
  board[m.f] = EMPTY;
  // Auto-Queen
  if (abs(p) == PAWN) {
    if (m.t >= 56 || m.t <= 7)
      board[m.t] = (p > 0 ? QUEEN : -QUEEN);
  }
}

int search(int depth, int alpha, int beta) {
  nodes++;
  if (depth == 0)
    return evaluate();

  Move moves[128];
  int num = gen_moves(moves);
  if (num == 0)
    return (side == WHITE) ? -10000 : 10000;

  // King Capture Check
  int k_found = 0, K_found = 0;
  for (int i = 0; i < 64; i++) {
    if (board[i] == KING)
      K_found = 1;
    if (board[i] == -KING)
      k_found = 1;
  }
  if (!K_found)
    return -20000;
  if (!k_found)
    return 20000;

  if (side == WHITE) {
    int max = -INF;
    for (int i = 0; i < num; i++) {
      int sv_t = board[moves[i].t];
      int sv_f = board[moves[i].f];
      apply(moves[i]);
      side = BLACK;
      int val = search(depth - 1, alpha, beta);
      side = WHITE;
      board[moves[i].f] = sv_f;
      board[moves[i].t] = sv_t;
      if (val > max)
        max = val;
      if (val > alpha)
        alpha = val;
      if (beta <= alpha)
        break;
    }
    return max;
  } else {
    int min = INF;
    for (int i = 0; i < num; i++) {
      int sv_t = board[moves[i].t];
      int sv_f = board[moves[i].f];
      apply(moves[i]);
      side = WHITE;
      int val = search(depth - 1, alpha, beta);
      side = BLACK;
      board[moves[i].f] = sv_f;
      board[moves[i].t] = sv_t;
      if (val < min)
        min = val;
      if (val < beta)
        beta = val;
      if (beta <= alpha)
        break;
    }
    return min;
  }
}

Move best_move(int depth) {
  Move moves[128];
  int num = gen_moves(moves);
  Move best = moves[0];
  int best_val = (side == WHITE) ? -INF : INF;

  for (int i = 0; i < num; i++) {
    int r = rand_fast() % num;
    Move t = moves[i];
    moves[i] = moves[r];
    moves[r] = t;
  }

  printf(CLR_MOVE "Thinking (%d moves)... " CLR_RESET, num);

  for (int i = 0; i < num; i++) {
    int sv_t = board[moves[i].t];
    int sv_f = board[moves[i].f];
    apply(moves[i]);
    side = (side == WHITE) ? BLACK : WHITE;
    int val = search(depth - 1, -INF, INF);
    side = (side == WHITE) ? BLACK : WHITE;
    board[moves[i].f] = sv_f;
    board[moves[i].t] = sv_t;

    if (side == WHITE) {
      if (val > best_val) {
        best_val = val;
        best = moves[i];
      }
    } else {
      if (val < best_val) {
        best_val = val;
        best = moves[i];
      }
    }
  }
  printf("\n");
  return best;
}

int parse(char *s) {
  if (s[0] < 'a' || s[0] > 'h' || s[1] < '1' || s[1] > '8')
    return -1;
  return (s[1] - '1') * 8 + (s[0] - 'a');
}

int main() {
  init_board();
  char buf[32];

  while (1) {
    print_board();

    int k = 0, K = 0;
    for (int i = 0; i < 64; i++) {
      if (board[i] == KING)
        K = 1;
      if (board[i] == -KING)
        k = 1;
    }
    if (!K) {
      printf(CLR_B_PIECE "Black Wins!\n" CLR_RESET);
      break;
    }
    if (!k) {
      printf(CLR_W_PIECE "White Wins!\n" CLR_RESET);
      break;
    }

    if (side == WHITE) {
      printf("Move (e.g. e2e4): ");
      gets(buf, 32);
      if (buf[0] == 'q')
        break;

      int f = parse(buf);
      int t = parse(buf + 2);
      if (f == -1 || t == -1)
        continue;

      Move moves[128];
      int num = gen_moves(moves);
      int ok = 0;
      for (int i = 0; i < num; i++) {
        if (moves[i].f == f && moves[i].t == t)
          ok = 1;
      }

      if (ok) {
        apply((Move){f, t, 0});
        side = BLACK;
      } else {
        printf("Illegal move.\n");
        for (volatile int x = 0; x < 5000000; x++)
          ;
      }
    } else {
      Move m = best_move(3);
      apply(m);
      printf("AI: %c%d%c%d\n", (m.f % 8) + 'a', (m.f / 8) + 1, (m.t % 8) + 'a',
             (m.t / 8) + 1);
      side = WHITE;
      for (volatile int x = 0; x < 10000000; x++)
        ;
    }
  }
  return 0;
}
