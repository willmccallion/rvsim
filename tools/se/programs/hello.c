/* The first thing anyone runs: printf, argv, the environment, the exit code. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    printf("hello from %s with %d arguments\n", argv[0], argc - 1);
    for (int i = 1; i < argc; i++) {
        printf("  argv[%d] = %s\n", i, argv[i]);
    }
    const char *home = getenv("SE_GREETING");
    printf("SE_GREETING = %s\n", home ? home : "(unset)");
    return argc > 1 ? atoi(argv[1]) : 0;
}
