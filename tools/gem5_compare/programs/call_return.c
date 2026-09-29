#include "common.h"
/* Deep recursion: calls and returns through the return address stack. */
__attribute__((noinline)) static u64 fib(u64 n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
int main(void) {
    sink = fib(22);
    return 0;
}
