// Marks a resume point and a measured region with rvsim.h: the host can
// stop at the break, switch configuration, and measure the work between
// the two stats snapshots.
#include "rvsim.h"
#include "stdio.h"

#define RESUME_POINT 1
#define WORK_START 10
#define WORK_END 11

static volatile unsigned long setup_n = 2000;
static volatile unsigned long work_n = 20000;

static unsigned long sum_of_squares(unsigned long n) {
  unsigned long sum = 0;
  for (unsigned long i = 0; i < n; i++)
    sum += i * i;
  return sum;
}

int main(void) {
  unsigned long warm = sum_of_squares(setup_n);
  printf("setup %lu\n", warm);
  rvsim_break(RESUME_POINT);
  rvsim_dump_stats(WORK_START);
  unsigned long work = sum_of_squares(work_n);
  rvsim_dump_stats(WORK_END);
  printf("work %lu\n", work);
  return 0;
}
