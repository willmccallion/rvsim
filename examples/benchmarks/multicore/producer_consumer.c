// Hart 0 produces into a ring buffer, hart 1 consumes and checksums.
#include "smp.h"
#include "stdio.h"

#define ITEMS 4000
#define RING 16

static volatile uint64_t ring[RING];
static volatile uint64_t head;
static volatile uint64_t tail;
static volatile uint64_t consumer_sum;
static smp_barrier_t barrier;

static uint64_t item_value(uint64_t i) { return i * 2654435761ull + 12345; }

int smp_main(unsigned long hart, unsigned long harts) {
  smp_barrier(&barrier, harts);
  if (hart == 0) {
    for (uint64_t i = 0; i < ITEMS; i++) {
      while (head - tail == RING) {
      }
      ring[head % RING] = item_value(i);
      smp_fence_w();
      head = head + 1;
    }
  } else if (hart == 1) {
    uint64_t sum = 0;
    for (uint64_t i = 0; i < ITEMS; i++) {
      while (head == tail) {
      }
      smp_fence_r();
      sum += ring[tail % RING];
      smp_fence();
      tail = tail + 1;
    }
    consumer_sum = sum;
  }
  smp_barrier(&barrier, harts);
  if (hart != 0) {
    return 0;
  }
  uint64_t expected = 0;
  for (uint64_t i = 0; i < ITEMS; i++) {
    expected += item_value(i);
  }
  printf("producer_consumer: %d items, sum=%lu expected=%lu\n", ITEMS, consumer_sum, expected);
  return consumer_sum == expected ? 0 : 1;
}
