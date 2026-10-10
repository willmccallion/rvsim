#ifndef HTIF_H
#define HTIF_H

#include <stdint.h>

#define HTIF_SYS_READ 63
#define HTIF_SYS_WRITE 64
#define HTIF_SYS_EXIT 93

/* Performs `which` on the host with up to three arguments and returns
 * its result, or the negated errno. */
uint64_t htif_syscall(uint64_t which, uint64_t arg0, uint64_t arg1, uint64_t arg2);

/* Ends the run with `code`. */
void htif_exit(int code) __attribute__((noreturn));

#endif
