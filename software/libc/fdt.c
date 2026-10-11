#include "fdt.h"

#include <stdint.h>

#define FDT_MAGIC 0xd00dfeed
#define FDT_BEGIN_NODE 1
#define FDT_END_NODE 2
#define FDT_PROP 3
#define FDT_NOP 4

static uint32_t be32(const uint8_t *p) {
  return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

static int starts_with(const char *s, const char *prefix) {
  while (*prefix) {
    if (*s++ != *prefix++) {
      return 0;
    }
  }
  return 1;
}

static unsigned long padded_string_len(const char *s) {
  unsigned long len = 0;
  while (s[len]) {
    len++;
  }
  return (len + 1 + 3) & ~3UL;
}

unsigned long fdt_cpu_count(const void *fdt) {
  const uint8_t *base = fdt;
  if (fdt == 0 || be32(base) != FDT_MAGIC) {
    return 1;
  }
  const uint8_t *p = base + be32(base + 8);
  unsigned long cpus = 0;
  int depth = 0;
  int in_cpus = 0;
  for (;;) {
    uint32_t token = be32(p);
    p += 4;
    switch (token) {
    case FDT_BEGIN_NODE: {
      const char *name = (const char *)p;
      depth++;
      if (depth == 2) {
        in_cpus = starts_with(name, "cpus") && name[4] == 0;
      } else if (depth == 3 && in_cpus && starts_with(name, "cpu@")) {
        cpus++;
      }
      p += padded_string_len(name);
      break;
    }
    case FDT_END_NODE:
      depth--;
      break;
    case FDT_PROP:
      p += 8 + ((be32(p) + 3) & ~3U);
      break;
    case FDT_NOP:
      break;
    default:
      return cpus ? cpus : 1;
    }
  }
}
