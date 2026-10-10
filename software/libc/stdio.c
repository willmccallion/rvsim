#include "stdio.h"

#ifdef RVSIM_IO_HTIF
#include "htif.h"

/* Output goes to the host through HTIF write syscalls, a line at a time. */
static char out_buf[256];
static unsigned out_len;

void stdio_flush(void) {
  if (out_len) {
    htif_syscall(HTIF_SYS_WRITE, 1, (uint64_t)(uintptr_t)out_buf, out_len);
    out_len = 0;
  }
}

void putchar(char c) {
  out_buf[out_len++] = c;
  if (c == '\n' || out_len == sizeof out_buf) {
    stdio_flush();
  }
}

char getchar(void) {
  char c;
  stdio_flush();
  if (htif_syscall(HTIF_SYS_READ, 0, (uint64_t)(uintptr_t)&c, 1) != 1) {
    return 0;
  }
  return c;
}
#else
void stdio_flush(void) {}

// Low-level Output
void putchar(char c) {
  *(volatile unsigned char *)UART_BASE = (unsigned char)c;
}

char getchar(void) {
  // Poll LSR (Line Status Register) at offset 5 until data is ready
  // LSR bit 0 (0x01) = Data Ready (DR)
  volatile unsigned char *lsr = (volatile unsigned char *)(UART_BASE + 5);
  while ((*lsr & 0x01) == 0) {
    // Wait for data to be available (blocking)
  }
  // Read from RBR (Receiver Buffer Register) at offset 0
  return *(volatile char *)UART_BASE;
}
#endif

void puts(const char *s) {
  while (*s) {
    putchar(*s++);
  }
  putchar('\n');
}

// Helper for numbers
static void print_num(long long n, int base, int sign) {
  char buf[32];
  int i = 0;
  unsigned long long u = n;

  if (sign && (n < 0)) {
    putchar('-');
    u = -n;
  }

  if (u == 0) {
    putchar('0');
    return;
  }

  while (u > 0) {
    int rem = u % base;
    buf[i++] = (rem < 10) ? (rem + '0') : (rem - 10 + 'a');
    u /= base;
  }

  while (i-- > 0) {
    putchar(buf[i]);
  }
}

static void print_double(double v, int precision) {
  if (v < 0) {
    putchar('-');
    v = -v;
  }

  long int_part = (long)v;
  double remainder = v - int_part;

  // Print Integer part
  print_num(int_part, 10, 0);
  putchar('.');

  // Print Fractional part
  while (precision-- > 0) {
    remainder *= 10.0;
    int digit = (int)remainder;
    putchar(digit + '0');
    remainder -= digit;
  }
}

// The printf logic
void printf(const char *fmt, ...) {
  va_list args;
  va_start(args, fmt);

  for (const char *p = fmt; *p; p++) {
    if (*p != '%') {
      putchar(*p);
      continue;
    }

    p++; // Skip '%'

    // Check for length modifier 'l' (long)
    int is_long = 0;
    if (*p == 'l') {
      is_long = 1;
      p++;
      // Handle 'll' (long long) - treat same as long for 64-bit
      if (*p == 'l') {
        p++;
      }
    }

    switch (*p) {
    case 'c': {
      int c = va_arg(args, int);
      putchar(c);
      break;
    }
    case 's': {
      const char *s = va_arg(args, const char *);
      if (!s)
        s = "(null)";
      while (*s)
        putchar(*s++);
      break;
    }
    case 'd': {
      long long d;
      if (is_long) {
        d = va_arg(args, long);
      } else {
        d = va_arg(args, int);
      }
      print_num(d, 10, 1);
      break;
    }
    case 'u': {
      unsigned long long u;
      if (is_long) {
        u = va_arg(args, unsigned long);
      } else {
        u = va_arg(args, unsigned int);
      }
      print_num(u, 10, 0);
      break;
    }
    case 'x': {
      unsigned long long x;
      if (is_long) {
        x = va_arg(args, unsigned long);
      } else {
        x = va_arg(args, unsigned int);
      }
      print_num(x, 16, 0);
      break;
    }
    case 'f': {
      double f = va_arg(args, double); // float is promoted to double in varargs
      print_double(f, 6);              // Default to 6 decimal places
      break;
    }
    case '%': {
      putchar('%');
      break;
    }
    default: {
      putchar('%');
      if (is_long)
        putchar('l');
      putchar(*p);
      break;
    }
    }
  }

  va_end(args);
}

int gets(char *buf, int max_len) {
  int i = 0;
  char c;
  while (i < max_len - 1) {
    c = getchar();

    if (c == '\n' || c == '\r') {
      break;
    }

    buf[i++] = c;
  }
  buf[i] = '\0'; // Null terminate
  return i;
}

int strcmp(const char *s1, const char *s2) {
  while (*s1 && (*s1 == *s2)) {
    s1++;
    s2++;
  }
  return *(const unsigned char *)s1 - *(const unsigned char *)s2;
}

int atoi(const char *str) {
  int res = 0;
  while (*str >= '0' && *str <= '9') {
    res = (res << 3) + (res << 1) + (*str - '0');
    str++;
  }
  return res;
}
