/* The heap: brk through small allocations, mmap through large ones,
 * realloc through mremap, and the pattern checked after every step. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static unsigned checksum(const unsigned char *p, size_t n) {
    unsigned sum = 0;
    for (size_t i = 0; i < n; i++) sum = sum * 31 + p[i];
    return sum;
}

int main(void) {
    enum { SMALL = 64, LARGE = 4 << 20, COUNT = 1000 };
    unsigned char *small[COUNT];
    for (int i = 0; i < COUNT; i++) {
        small[i] = malloc(SMALL);
        memset(small[i], i & 0xff, SMALL);
    }
    unsigned char *large = malloc(LARGE);
    for (size_t i = 0; i < LARGE; i++) large[i] = (unsigned char)(i * 7);
    printf("large checksum %08x\n", checksum(large, LARGE));
    large = realloc(large, LARGE * 2);
    printf("after realloc %08x\n", checksum(large, LARGE));
    unsigned total = 0;
    for (int i = 0; i < COUNT; i++) {
        total += checksum(small[i], SMALL);
        free(small[i]);
    }
    printf("small checksum %08x\n", total);
    free(large);
    return 0;
}
