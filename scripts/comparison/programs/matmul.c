#include "common.h"
/* 48x48 double matrix multiply. */
#define N 48
static double a[N][N], b[N][N], c[N][N];
int main(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) { a[i][j] = i + j; b[i][j] = i - j; }
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) {
            double s = 0;
            for (int k = 0; k < N; k++) s += a[i][k] * b[k][j];
            c[i][j] = s;
        }
    sink = (u64)c[N / 2][N / 3];
    return 0;
}
