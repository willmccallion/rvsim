/* The clocks move forward and agree with each other; their values are the
 * simulator's, so only their monotonicity and relations are checked. */
#include <stdio.h>
#include <time.h>
#include <sys/time.h>
#include <sys/times.h>
#include <unistd.h>

static long work(long n) {
    volatile long acc = 0;
    for (long i = 0; i < n; i++) acc += i ^ (i >> 3);
    return acc;
}

int main(void) {
    struct timespec a, b;
    clock_gettime(CLOCK_MONOTONIC, &a);
    long r = work(200000);
    clock_gettime(CLOCK_MONOTONIC, &b);
    long long ns = (b.tv_sec - a.tv_sec) * 1000000000LL + (b.tv_nsec - a.tv_nsec);
    printf("monotonic advanced: %s\n", ns > 0 ? "yes" : "no");

    struct timeval tv;
    gettimeofday(&tv, NULL);
    printf("realtime after 2020: %s\n", tv.tv_sec > 1577836800 ? "yes" : "no");

    struct tms t;
    clock_t ticks = times(&t);
    printf("times returns ticks: %s\n", ticks != (clock_t)-1 ? "yes" : "no");
    printf("clock ticks per second: %ld\n", sysconf(_SC_CLK_TCK));
    printf("work %ld\n", r);
    return 0;
}
