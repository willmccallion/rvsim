#include "bench.h"
#include "stdio.h"
#include "stdlib.h"

#define WINDOW_SIZE 128
#define LOOKAHEAD_BUFFER 16
#define INPUT_SIZE 4096

unsigned char input[INPUT_SIZE];
unsigned char output[INPUT_SIZE * 2]; // Worst case expansion

void init_data() {
  // Generate repetitive data to make compression meaningful
  const char *dict = "The quick brown fox jumps over the lazy dog. ";
  int dict_len = 45;
  for (int i = 0; i < INPUT_SIZE; i++) {
    input[i] = dict[i % dict_len];
  }
}

int lz77_compress(unsigned char *src, int src_len, unsigned char *dst) {
  int dst_idx = 0;
  int src_idx = 0;

  while (src_idx < src_len) {
    int best_match_len = 0;
    int best_match_dist = 0;

    // Search backward in window
    int start_search = (src_idx < WINDOW_SIZE) ? 0 : src_idx - WINDOW_SIZE;

    for (int i = start_search; i < src_idx; i++) {
      int len = 0;
      while (len < LOOKAHEAD_BUFFER && (src_idx + len) < src_len &&
             src[i + len] == src[src_idx + len]) {
        len++;
      }

      if (len > best_match_len) {
        best_match_len = len;
        best_match_dist = src_idx - i;
      }
    }

    if (best_match_len >= 3) {
      // Write (Distance, Length) pair marker
      // Simplified format: Flag (0x80) | Distance, Length
      dst[dst_idx++] = 0x80 | (best_match_dist & 0x7F);
      dst[dst_idx++] = best_match_len;
      src_idx += best_match_len;
    } else {
      // Literal
      dst[dst_idx++] = src[src_idx++];
    }
  }
  return dst_idx;
}

int main() {
  printf("LZ77 Compression Benchmark\n");
  init_data();

  unsigned long start = read_cycles();
  int comp_size = lz77_compress(input, INPUT_SIZE, output);
  unsigned long end = read_cycles();

  printf("Benchmark Cycles: %lu\n", end - start);
  printf("Original: %d, Compressed: %d\n", INPUT_SIZE, comp_size);
  printf("Compression Ratio: %d%%\n", (comp_size * 100) / INPUT_SIZE);

  return 0;
}
