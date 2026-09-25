#ifndef SMP_H
#define SMP_H

#include <stdint.h>

// Primitives for programs linked with crt0_smp.s: every hart runs
// smp_main(hart_id, hart_count); hart 0's return value is the exit code.

static inline unsigned long smp_hart_id(void) {
  unsigned long id;
  asm volatile("csrr %0, mhartid" : "=r"(id));
  return id;
}

static inline unsigned long smp_cycles(void) {
  unsigned long cycles;
  asm volatile("rdcycle %0" : "=r"(cycles));
  return cycles;
}

static inline void smp_fence(void) { asm volatile("fence rw, rw" ::: "memory"); }
static inline void smp_fence_w(void) { asm volatile("fence rw, w" ::: "memory"); }
static inline void smp_fence_r(void) { asm volatile("fence r, rw" ::: "memory"); }

static inline uint64_t smp_atomic_add(volatile uint64_t *addr, uint64_t val) {
  uint64_t old;
  asm volatile("amoadd.d %0, %2, (%1)" : "=r"(old) : "r"(addr), "r"(val) : "memory");
  return old;
}

static inline uint32_t smp_atomic_swap_acquire(volatile uint32_t *addr, uint32_t val) {
  uint32_t old;
  asm volatile("amoswap.w.aq %0, %2, (%1)" : "=r"(old) : "r"(addr), "r"(val) : "memory");
  return old;
}

typedef struct {
  volatile uint32_t locked;
} smp_lock_t;

static inline void smp_lock(smp_lock_t *lock) {
  while (smp_atomic_swap_acquire(&lock->locked, 1) != 0) {
  }
}

static inline void smp_unlock(smp_lock_t *lock) {
  asm volatile("amoswap.w.rl zero, zero, (%0)" : : "r"(&lock->locked) : "memory");
}

// Sense-reversing barrier for a fixed number of harts.
typedef struct {
  volatile uint64_t arrived;
  volatile uint64_t generation;
} smp_barrier_t;

static inline void smp_barrier(smp_barrier_t *barrier, unsigned long harts) {
  uint64_t generation = barrier->generation;
  if (smp_atomic_add(&barrier->arrived, 1) + 1 == harts) {
    barrier->arrived = 0;
    smp_fence_w();
    barrier->generation = generation + 1;
  } else {
    while (barrier->generation == generation) {
    }
  }
  smp_fence();
}

#endif
