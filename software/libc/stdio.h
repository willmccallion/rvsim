#ifndef STDIO_H
#define STDIO_H

#include <stdarg.h>

// The base address of your UART in the simulator
#define UART_BASE 0x10000000UL

void putchar(char c);
void puts(const char *s);
void printf(const char *fmt, ...);

char getchar(void);
void putchar(char c);

int gets(char *buf, int max_len);
int strcmp(const char *s1, const char *s2);
int atoi(const char *str);

#endif
