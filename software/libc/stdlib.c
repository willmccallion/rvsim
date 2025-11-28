#include "stdlib.h"
#include "stdio.h"

// Defined in user.ld
extern char _end;

struct header {
  size_t size;
  struct header *next;
};

// Alignment must be a power of 2
#define ALIGN_SIZE 8
#define ALIGN(x) (((x) + (ALIGN_SIZE - 1)) & ~(ALIGN_SIZE - 1))
#define BLOCK_SIZE sizeof(struct header)

static struct header *free_list = NULL;
static char *heap_top = NULL;

static void *sbrk(long increment) {
  if (heap_top == NULL) {
    heap_top = &_end;
    unsigned long addr = (unsigned long)heap_top;
    if (addr % ALIGN_SIZE != 0) {
      heap_top += (ALIGN_SIZE - (addr % ALIGN_SIZE));
    }
  }

  char *old_top = heap_top;

  // Hard limit check
  // 0x80200000 (Load) + 0x1000000 (16MB) = 0x81200000
  if ((unsigned long)(heap_top + increment) >= 0x81200000) {
    return (void *)-1;
  }

  heap_top += increment;
  return (void *)old_top;
}

void free(void *ptr) {
  if (!ptr)
    return;

  // Point back to the header
  struct header *block = (struct header *)ptr - 1;

  block->next = free_list;
  free_list = block;
}

void *malloc(size_t size) {
  if (size == 0)
    return NULL;

  size_t total_size = ALIGN(size + BLOCK_SIZE);

  struct header *prev = NULL;
  struct header *curr = free_list;

  while (curr) {
    if (curr->size >= total_size) {
      if (curr->size >= total_size + BLOCK_SIZE + ALIGN_SIZE) {
        struct header *remaining = (struct header *)((char *)curr + total_size);
        remaining->size = curr->size - total_size;
        remaining->next = curr->next;

        curr->size = total_size;

        if (prev) {
          prev->next = remaining;
        } else {
          free_list = remaining;
        }
      } else {
        if (prev) {
          prev->next = curr->next;
        } else {
          free_list = curr->next;
        }
      }

      return (void *)(curr + 1);
    }
    prev = curr;
    curr = curr->next;
  }

  struct header *block = (struct header *)sbrk(total_size);
  if (block == (void *)-1) {
    printf("malloc: Out of memory! (Request: %d bytes)\n", (int)size);
    return NULL;
  }

  block->size = total_size;

  return (void *)(block + 1);
}
