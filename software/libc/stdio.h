#ifndef STDIO_H
#define STDIO_H

#include <stdarg.h>

#define UART_BASE 0x10000000UL

void putchar(char c);
/* Writes buffered output to the host (a no-op on the UART). */
void stdio_flush(void);
void puts(const char *s);
void printf(const char *fmt, ...);

char getchar(void);
void putchar(char c);

int gets(char *buf, int max_len);
int strcmp(const char *s1, const char *s2);
int atoi(const char *str);

#endif
