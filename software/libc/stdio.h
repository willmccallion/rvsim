#ifndef STDIO_H
#define STDIO_H

/* The part of C's stdio the bare-metal programs use, over HTIF or the UART. */

#include <stddef.h>

#define EOF (-1)

typedef struct FILE FILE;
extern FILE *stdin;
extern FILE *stdout;

int putchar(int c);
int puts(const char *s);
int printf(const char *fmt, ...);
int getchar(void);
char *fgets(char *s, int size, FILE *stream);
int fflush(FILE *stream);

#endif
