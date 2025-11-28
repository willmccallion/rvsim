#include <stdint.h>

#define UART_BASE 0x10000000
#define DISK_BASE 0x90000000
#define RAM_USER_BASE 0x80200000

#define UART_PTR ((volatile char *)UART_BASE)
#define DISK_PTR ((volatile uint8_t *)DISK_BASE)

#define ANSI_GREEN "\x1b[32m"
#define ANSI_RED "\x1b[31m"
#define ANSI_CYAN "\x1b[36m"
#define ANSI_RESET "\x1b[0m"

#define KERNEL_SIZE 16384

struct FileHeader {
  char name[32];
  uint32_t offset;
  uint32_t size;
};

extern long switch_to_user(uint64_t entry_point);

void kputc(char c) { *UART_PTR = c; }

void kprint(const char *s) {
  while (*s)
    kputc(*s++);
}

// Helper to print long integers
void kprint_long(long n) {
  if (n == 0) {
    kputc('0');
    return;
  }
  if (n < 0) {
    kputc('-');
    n = -n;
  }
  char buf[20];
  int i = 0;
  while (n > 0) {
    buf[i++] = (n % 10) + '0';
    n /= 10;
  }
  while (i > 0)
    kputc(buf[--i]);
}

// Helper to print hex
void kprint_hex(uint64_t n) {
  kprint("0x");
  char hex[] = "0123456789abcdef";
  for (int i = 60; i >= 0; i -= 4) {
    int nibble = (n >> i) & 0xF;
    kputc(hex[nibble]);
  }
}

char kgetc() { return *UART_PTR; }

void kgets(char *buf, int max) {
  int i = 0;
  while (i < max - 1) {
    char c = kgetc();
    if (c == 0)
      continue;

    // Handle Backspace
    if (c == 127 || c == '\b') {
      if (i > 0) {
        i--;
      }
      continue;
    }

    if (c == '\n' || c == '\r')
      break;

    buf[i++] = c;
  }
  buf[i] = 0;
  kputc('\n');
}

int kstrcmp(const char *s1, const char *s2) {
  while (*s1 && (*s1 == *s2)) {
    s1++;
    s2++;
  }
  return *(const unsigned char *)s1 - *(const unsigned char *)s2;
}

void kmemcpy(void *dest, const void *src, uint32_t n) {
  uint8_t *d = (uint8_t *)dest;
  const uint8_t *s = (const uint8_t *)src;
  while (n--)
    *d++ = *s++;
}

void kmemset(void *dest, uint8_t val, uint32_t n) {
  uint8_t *d = (uint8_t *)dest;
  while (n--)
    *d++ = val;
}

void print_banner() {
  kprint("\n");
  kprint(ANSI_CYAN "RISC-V MicroKernel v2.0.0" ANSI_RESET "\n");
  kprint("Build: " __DATE__ " " __TIME__ "\n");
  kprint("CPUs: 1 | RAM: 128MB | Arch: rv64im\n\n");

  kprint("[ " ANSI_GREEN "OK" ANSI_RESET " ] Initializing UART...\n");
  kprint("[ " ANSI_GREEN "OK" ANSI_RESET " ] Mounting Virtual Disk...\n");
  kprint("[ " ANSI_GREEN "OK" ANSI_RESET " ] Clearing User Memory...\n");
  kprint("[ " ANSI_GREEN "OK" ANSI_RESET " ] System Ready.\n\n");
}

void cmd_ls(uint32_t count, struct FileHeader *headers) {
  kprint("PERM   SIZE    NAME\n");
  kprint("----   ----    ----\n");
  for (uint32_t i = 0; i < count; i++) {
    kprint("-r-x   ");

    uint32_t s = headers[i].size;
    if (s < 1000)
      kprint(" ");
    if (s < 100)
      kprint(" ");
    if (s < 10)
      kprint(" ");

    kprint_long(s);

    kprint("    ");
    kprint(headers[i].name);
    kprint("\n");
  }
}

void kmain() {
  print_banner();

  uint32_t *file_count_ptr = (uint32_t *)(DISK_PTR + KERNEL_SIZE);
  uint32_t file_count = *file_count_ptr;
  struct FileHeader *headers =
      (struct FileHeader *)(DISK_PTR + KERNEL_SIZE + 4);

  long last_exit_code = 0;

  while (1) {
    kprint(ANSI_GREEN "root@riscv" ANSI_RESET ":" ANSI_CYAN "~" ANSI_RESET);

    if (last_exit_code != 0) {
      kprint(ANSI_RED " (");
      kprint_long(last_exit_code);
      kprint(")" ANSI_RESET);
      last_exit_code = 0;
    }

    kprint("# ");

    char cmd[32];
    kgets(cmd, 32);

    if (cmd[0] == 0)
      continue;

    if (kstrcmp(cmd, "ls") == 0) {
      cmd_ls(file_count, headers);
      continue;
    }

    if (kstrcmp(cmd, "help") == 0) {
      kprint("Built-ins: ls, help, clear, exit\n");
      continue;
    }

    if (kstrcmp(cmd, "clear") == 0) {
      kprint("\x1b[2J\x1b[H");
      continue;
    }

    if (kstrcmp(cmd, "exit") == 0) {
      kprint("[" ANSI_GREEN " OK " ANSI_RESET "] "
             "System halting.\n");
      asm volatile("li a7, 93\n li a0, 0\n nop\n nop\n nop\n ecall");
      while (1)
        ;
    }

    int found = -1;
    for (uint32_t i = 0; i < file_count; i++) {
      if (kstrcmp(cmd, headers[i].name) == 0) {
        found = i;
        break;
      }
    }

    if (found != -1) {
      kmemset((void *)RAM_USER_BASE, 0, 0x100000);

      uint8_t *src = (uint8_t *)(DISK_PTR + headers[found].offset);
      uint8_t *dst = (uint8_t *)RAM_USER_BASE;
      kmemcpy(dst, src, headers[found].size);

      long code = switch_to_user(RAM_USER_BASE);

      if (code >= 0 && code <= 255) {
        last_exit_code = code;
      } else {
        kprint("\n" ANSI_RED "[FATAL] Trap Cause: ");
        kprint_hex((uint64_t)code);
        kprint(ANSI_RESET "\n");
        last_exit_code = 139;
      }
    } else {
      kprint("sh: command not found: ");
      kprint(cmd);
      kprint("\n");
      last_exit_code = 127;
    }
  }
}
