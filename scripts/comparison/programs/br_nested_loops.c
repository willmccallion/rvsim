#include "common.h"
/* Short inner loops whose exits a predictor must learn. */
int main(void) {
    u64 acc = 0;
    for (int i = 0; i < 4000; i++)
        for (int j = 0; j < (i & 7) + 2; j++)
            for (int k = 0; k < 3; k++) acc += i ^ j ^ k;
    sink = acc;
    return 0;
}
