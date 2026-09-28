#include "common.h"
/* Quicksort of 8192 pseudo-random integers. */
#define N 8192
static u64 v[N];
static void sort(u64 *x, i64 lo, i64 hi) {
    while (lo < hi) {
        u64 pivot = x[(lo + hi) / 2];
        i64 i = lo, j = hi;
        while (i <= j) {
            while (x[i] < pivot) i++;
            while (x[j] > pivot) j--;
            if (i <= j) { u64 t = x[i]; x[i] = x[j]; x[j] = t; i++; j--; }
        }
        if (j - lo < hi - i) { sort(x, lo, j); lo = i; } else { sort(x, i, hi); hi = j; }
    }
}
int main(void) {
    u64 s = 7;
    for (int i = 0; i < N; i++) v[i] = lcg(&s);
    sort(v, 0, N - 1);
    sink = v[N / 2];
    return 0;
}
