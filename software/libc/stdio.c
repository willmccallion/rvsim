#include "stdio.h"

#include <stdarg.h>

#include "htif.h"

struct FILE {
  int fd;
};

static struct FILE console[] = {{0}, {1}};
FILE *stdin = &console[0];
FILE *stdout = &console[1];

/* Output goes to the host through HTIF write syscalls, a line at a time. */
static char out_buf[256];
static unsigned out_len;

int fflush(FILE *stream) {
  (void)stream;
  if (out_len) {
    htif_syscall(HTIF_SYS_WRITE, 1, (uint64_t)(uintptr_t)out_buf, out_len);
    out_len = 0;
  }
  return 0;
}

int putchar(int c) {
  out_buf[out_len++] = (char)c;
  if (c == '\n' || out_len == sizeof out_buf) {
    fflush(stdout);
  }
  return (unsigned char)c;
}

int getchar(void) {
  char c;
  fflush(stdout);
  if (htif_syscall(HTIF_SYS_READ, 0, (uint64_t)(uintptr_t)&c, 1) != 1) {
    return EOF;
  }
  return (unsigned char)c;
}

char *fgets(char *s, int size, FILE *stream) {
  (void)stream;
  int n = 0;
  while (n < size - 1) {
    int c = getchar();
    if (c == EOF) {
      break;
    }
    s[n++] = (char)c;
    if (c == '\n') {
      break;
    }
  }
  if (n == 0) {
    return NULL;
  }
  s[n] = '\0';
  return s;
}

int puts(const char *s) {
  while (*s) {
    putchar(*s++);
  }
  putchar('\n');
  return 0;
}

static int printed;

static void emit(int c) {
  putchar(c);
  printed++;
}

static void print_num(long long n, int base, int sign) {
  char buf[32];
  int i = 0;
  unsigned long long u = n;

  if (sign && (n < 0)) {
    emit('-');
    u = -n;
  }

  if (u == 0) {
    emit('0');
    return;
  }

  while (u > 0) {
    int rem = u % base;
    buf[i++] = (rem < 10) ? (rem + '0') : (rem - 10 + 'a');
    u /= base;
  }

  while (i-- > 0) {
    emit(buf[i]);
  }
}

static void print_double(double v, int precision) {
  if (v < 0) {
    emit('-');
    v = -v;
  }

  long int_part = (long)v;
  double remainder = v - int_part;

  print_num(int_part, 10, 0);
  emit('.');

  while (precision-- > 0) {
    remainder *= 10.0;
    int digit = (int)remainder;
    emit(digit + '0');
    remainder -= digit;
  }
}

/* %c %s %d %u %x %f with an optional l or ll; anything else is printed as is. */
int printf(const char *fmt, ...) {
  va_list args;
  va_start(args, fmt);
  printed = 0;

  for (const char *p = fmt; *p; p++) {
    if (*p != '%') {
      emit(*p);
      continue;
    }

    p++;

    int is_long = 0;
    if (*p == 'l') {
      is_long = 1;
      p++;
      if (*p == 'l') {
        p++;
      }
    }

    switch (*p) {
    case 'c': {
      int c = va_arg(args, int);
      emit(c);
      break;
    }
    case 's': {
      const char *s = va_arg(args, const char *);
      if (!s)
        s = "(null)";
      while (*s)
        emit(*s++);
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
      double f = va_arg(args, double);
      print_double(f, 6);
      break;
    }
    case '%': {
      emit('%');
      break;
    }
    default: {
      emit('%');
      if (is_long)
        emit('l');
      emit(*p);
      break;
    }
    }
  }

  va_end(args);
  return printed;
}
