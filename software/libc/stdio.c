#include "stdio.h"

// Low-level Output
void putchar(char c) {
  *(volatile unsigned char *)UART_BASE = (unsigned char)c;
}

char getchar(void) {
  // Read from memory-mapped UART
  return *(volatile char *)UART_BASE;
}

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
    switch (*p) {
    case 'c': {
      // char is promoted to int in varargs
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
      int d = va_arg(args, int);
      print_num(d, 10, 1);
      break;
    }
    case 'u': {
      unsigned int u = va_arg(args, unsigned int);
      print_num(u, 10, 0);
      break;
    }
    case 'x': {
      unsigned int x = va_arg(args, unsigned int);
      print_num(x, 16, 0);
      break;
    }
    case '%': {
      putchar('%');
      break;
    }
    default: {
      // Unknown specifier, just print it
      putchar('%');
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
