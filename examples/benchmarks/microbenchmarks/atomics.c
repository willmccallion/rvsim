#include <stdint.h>

#define UART_BASE 0x10000000
#define TEST_ADDR 0x80001000UL

void print_char(char c) {
  volatile uint8_t *uart = (uint8_t *)UART_BASE;
  *uart = c;
}

void print_str(const char *s) {
  while (*s) {
    print_char(*s++);
  }
}

void exit(int code) {
  register int a0 asm("a0") = code;
  register int a7 asm("a7") = 93; // Syscall Exit
  asm volatile("ecall" : : "r"(a0), "r"(a7));
  while (1)
    ;
}

void fail(const char *msg) {
  print_str("[FAIL] ");
  print_str(msg);
  print_char('\n');
  exit(1);
}

static inline uint32_t amoadd_w(volatile uint32_t *addr, uint32_t val) {
  uint32_t ret;
  asm volatile("amoadd.w %0, %2, (%1)"
               : "=r"(ret)
               : "r"(addr), "r"(val)
               : "memory");
  return ret;
}

static inline uint32_t amoswap_w(volatile uint32_t *addr, uint32_t val) {
  uint32_t ret;
  asm volatile("amoswap.w %0, %2, (%1)"
               : "=r"(ret)
               : "r"(addr), "r"(val)
               : "memory");
  return ret;
}

static inline uint32_t lr_w(volatile uint32_t *addr) {
  uint32_t ret;
  asm volatile("lr.w %0, (%1)" : "=r"(ret) : "r"(addr) : "memory");
  return ret;
}

static inline uint32_t sc_w(volatile uint32_t *addr, uint32_t val) {
  uint32_t ret;
  asm volatile("sc.w %0, %2, (%1)"
               : "=r"(ret)
               : "r"(addr), "r"(val)
               : "memory");
  return ret;
}

int main() {
  volatile uint32_t *mem = (volatile uint32_t *)TEST_ADDR;

  print_str("[TEST] Starting Atomics Test (C Version)\n");

  *mem = 10;
  uint32_t old = amoadd_w(mem, 5);

  if (old != 10)
    fail("AMOADD returned incorrect old value");
  if (*mem != 15)
    fail("AMOADD did not update memory correctly");
  print_str("  AMOADD: Pass\n");

  *mem = 15;
  old = amoswap_w(mem, 99);

  if (old != 15)
    fail("AMOSWAP returned incorrect old value");
  if (*mem != 99)
    fail("AMOSWAP did not update memory correctly");
  print_str("  AMOSWAP: Pass\n");

  *mem = 100;
  uint32_t val = lr_w(mem);
  if (val != 100)
    fail("LR read incorrect value");

  uint32_t sc_ret = sc_w(mem, 101);
  if (sc_ret != 0)
    fail("SC failed (returned non-zero) on uncontested access");
  if (*mem != 101)
    fail("SC success but memory not updated");
  print_str("  LR/SC (Success): Pass\n");

  *mem = 50;
  val = lr_w(mem);

  *mem = 60;

  sc_ret = sc_w(mem, 70);
  if (sc_ret == 0)
    fail("SC succeeded (returned 0) but reservation should be broken");
  if (*mem != 60)
    fail("SC failed but memory WAS updated");
  print_str("  LR/SC (Fail): Pass\n");

  print_str("[TEST] All Tests Passed\n");
  exit(0);
  return 0;
}
