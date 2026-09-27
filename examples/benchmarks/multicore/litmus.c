// Classic litmus tests on harts 0 and 1: message passing (with fences and
// with release/acquire atomics), load buffering with release/acquire, store
// buffering and coherent read-read. Forbidden outcomes fail the run; the
// allowed outcome counts are printed so a change in the memory model is
// visible.
#include "smp.h"
#include "stdio.h"

#define TRIALS 400

// On separate cache lines, so ordering cannot come from one line's coherence.
static volatile uint64_t x __attribute__((aligned(64)));
static volatile uint64_t y __attribute__((aligned(64)));
static volatile uint64_t r0, r1, r2;
static smp_barrier_t barrier;

static uint64_t mp_forbidden, mp_early, sb_both_zero, corr_forbidden, corr_stale;
static uint64_t mp_ra_forbidden, lb_ra_forbidden;

// Store `val` with release semantics: every earlier access is performed first.
static inline void store_release(volatile uint64_t *addr, uint64_t val) {
  asm volatile("amoswap.d.rl zero, %1, (%0)" : : "r"(addr), "r"(val) : "memory");
}

// Load with acquire semantics: every later access is performed after it.
static inline uint64_t load_acquire(volatile uint64_t *addr) {
  uint64_t old;
  asm volatile("amoor.d.aq %0, zero, (%1)" : "=r"(old) : "r"(addr) : "memory");
  return old;
}

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

  // MP with release/acquire: x=1; y.rl=1  ||  while(!y.aq); r=x  =>  r=1.
  for (int t = 0; t < TRIALS; t++) {
    if (hart == 0) {
      x = 0;
      y = 0;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0) {
      x = 1;
      store_release(&y, 1);
    } else if (hart == 1) {
      while (load_acquire(&y) == 0) {
      }
      r0 = x;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0 && r0 != 1) {
      mp_ra_forbidden++;
    }
  }

  // LB with release/acquire: r=x; y.rl=1  ||  while(!y.aq); x=1  =>  r=0.
  for (int t = 0; t < TRIALS; t++) {
    if (hart == 0) {
      x = 0;
      y = 0;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0) {
      r1 = x;
      store_release(&y, 1);
    } else if (hart == 1) {
      while (load_acquire(&y) == 0) {
      }
      x = 1;
    }
    smp_barrier(&barrier, harts);
    if (hart == 0 && r1 != 0) {
      lb_ra_forbidden++;
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
  printf("litmus: MP forbidden=%lu (early=%lu) MP-rel/acq forbidden=%lu LB-rel/acq forbidden=%lu "
         "SB both-zero=%lu CoRR forbidden=%lu (both-old=%lu)\n",
         mp_forbidden, mp_early, mp_ra_forbidden, lb_ra_forbidden, sb_both_zero, corr_forbidden,
         corr_stale);
  return (mp_forbidden == 0 && mp_ra_forbidden == 0 && lb_ra_forbidden == 0 && corr_forbidden == 0)
             ? 0
             : 1;
}
