// Classic litmus tests on harts 0 and 1: message passing, store buffering
// and coherent read-read. Forbidden outcomes fail the run; the allowed
// outcome counts are printed so a change in the memory model is visible.
#include "smp.h"
#include "stdio.h"

#define TRIALS 400

static volatile uint64_t x, y;
static volatile uint64_t r0, r1, r2;
static smp_barrier_t barrier;

static uint64_t mp_forbidden, mp_early, sb_both_zero, corr_forbidden, corr_stale;

int smp_main(unsigned long hart, unsigned long harts) {
  smp_barrier(&barrier, harts);

  // MP: x=1; fence; y=1  ||  while(!y); fence; r=x   =>  r must be 1.
  for (int t = 0; t < TRIALS; t++) {
    if (hart == 0) {
      x = 0;
      y = 0;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0) {
      x = 1;
      smp_fence_w();
      y = 1;
    } else if (hart == 1) {
      uint64_t spins = 0;
      while (y == 0) {
        spins++;
      }
      smp_fence_r();
      r0 = x;
      if (spins == 0) {
        mp_early++;
      }
    }
    smp_barrier(&barrier, harts);
    if (hart == 0 && r0 != 1) {
      mp_forbidden++;
    }
  }

  // SB: x=1; r=y  ||  y=1; r=x   =>  r0=r1=0 is allowed (store buffers).
  for (int t = 0; t < TRIALS; t++) {
    if (hart == 0) {
      x = 0;
      y = 0;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0) {
      x = 1;
      r0 = y;
    } else if (hart == 1) {
      y = 1;
      r1 = x;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0 && r0 == 0 && r1 == 0) {
      sb_both_zero++;
    }
  }

  // CoRR: x=1  ||  r1=x; r2=x   =>  r1=1, r2=0 is forbidden.
  for (int t = 0; t < TRIALS; t++) {
    if (hart == 0) {
      x = 0;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0) {
      x = 1;
    } else if (hart == 1) {
      r1 = x;
      r2 = x;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0) {
      if (r1 == 1 && r2 == 0) {
        corr_forbidden++;
      }
      if (r1 == 0 && r2 == 0) {
        corr_stale++;
      }
    }
  }

  if (hart != 0) {
    return 0;
  }
  printf("litmus: MP forbidden=%lu (early=%lu) SB both-zero=%lu CoRR forbidden=%lu (both-old=%lu)\n",
         mp_forbidden, mp_early, sb_both_zero, corr_forbidden, corr_stale);
  return (mp_forbidden == 0 && corr_forbidden == 0) ? 0 : 1;
}
