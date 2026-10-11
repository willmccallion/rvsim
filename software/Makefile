# Builds the bundled programs:
#   bin/programs, bin/benchmarks   static Linux user binaries, run under SE mode
#   bin/multicore                  bare-metal; every hart runs smp_main
#   bin-htif/programs, benchmarks  bare-metal builds that print and exit through
#                                  HTIF, for spike and the RTL simulators
# The Linux toolchain is $(SE_TARGET)-gcc, the bare-metal one $(TARGET)-gcc.

TARGET ?= riscv64-elf
SE_TARGET ?= riscv64-linux-gnu
CC = $(TARGET)-gcc
SE_CC = $(SE_TARGET)-gcc

BUILD_DIR = build
BIN = bin
BARE_BIN = bin-htif
LIB_DIR = libc
INC_DIR = include
LINUX_DIR = linux
PROG_DIR = ../examples/programs
BENCH_DIR = ../examples/benchmarks

ARCH_FLAGS = -march=rv64gcv -mabi=lp64d
SE_CFLAGS = $(ARCH_FLAGS) -std=gnu17 -static -g -I$(INC_DIR)
BARE_CFLAGS = $(ARCH_FLAGS) -mcmodel=medany -ffreestanding -nostdlib -g -I$(LIB_DIR) -I$(INC_DIR)
HTIF_CFLAGS = $(BARE_CFLAGS) -DRVSIM_IO_HTIF
# One RWX segment is the shape of these images; the linker need not say so.
BARE_LDFLAGS = -T $(LIB_DIR)/user.ld -Wl,--no-warn-rwx-segments

PROG_C_SRCS = $(wildcard $(PROG_DIR)/*.c)
PROG_S_SRCS = $(wildcard $(PROG_DIR)/*.s)
MICRO_SRCS = $(wildcard $(BENCH_DIR)/microbenchmarks/*.c)
SYNTH_SRCS = $(wildcard $(BENCH_DIR)/synthetic/*.c)
KERN_SRCS = $(wildcard $(BENCH_DIR)/kernels/*.c)
COMPLETE_SRCS = $(wildcard $(BENCH_DIR)/complete_prog/*.c)
BENCH_SRCS = $(MICRO_SRCS) $(SYNTH_SRCS) $(KERN_SRCS) $(COMPLETE_SRCS)
MULTICORE_SRCS = $(wildcard $(BENCH_DIR)/multicore/*.c)

SE_ELFS = \
	$(patsubst $(PROG_DIR)/%.c, $(BIN)/programs/%.elf, $(PROG_C_SRCS)) \
	$(patsubst $(PROG_DIR)/%.s, $(BIN)/programs/%.elf, $(PROG_S_SRCS)) \
	$(patsubst %.c, $(BIN)/benchmarks/%.elf, $(notdir $(BENCH_SRCS)))
MULTICORE_ELFS = $(patsubst $(BENCH_DIR)/multicore/%.c, $(BIN)/multicore/%.elf, $(MULTICORE_SRCS))
HTIF_ELFS = \
	$(patsubst $(PROG_DIR)/%.c, $(BARE_BIN)/programs/%.elf, $(PROG_C_SRCS)) \
	$(patsubst %.c, $(BARE_BIN)/benchmarks/%.elf, $(notdir $(BENCH_SRCS)))

UART_LIB_DIR = $(BUILD_DIR)/libc-uart
HTIF_LIB_DIR = $(BUILD_DIR)/libc-htif
LIBC_HEADERS = $(LIB_DIR)/stdio.h $(LIB_DIR)/stdlib.h $(LIB_DIR)/htif.h
UART_CRT0_SMP = $(UART_LIB_DIR)/crt0_smp.o
UART_LIBC = $(UART_LIB_DIR)/stdio.o $(UART_LIB_DIR)/stdlib.o
HTIF_LIBC = $(HTIF_LIB_DIR)/crt0_htif.o $(HTIF_LIB_DIR)/stdio.o $(HTIF_LIB_DIR)/stdlib.o $(HTIF_LIB_DIR)/htif.o

.PHONY: all dirs linux clean clean-no-linux

all: $(SE_ELFS) $(MULTICORE_ELFS) $(HTIF_ELFS)
	@echo "  Built $(words $(SE_ELFS) $(MULTICORE_ELFS) $(HTIF_ELFS)) ELFs"
	@echo "  SE mode:    $(BIN)/programs/ $(BIN)/benchmarks/"
	@echo "  Bare-metal: $(BIN)/multicore/ $(BARE_BIN)/programs/ $(BARE_BIN)/benchmarks/"

dirs:
	@mkdir -p $(UART_LIB_DIR) $(HTIF_LIB_DIR) \
	          $(BIN)/programs $(BIN)/benchmarks $(BIN)/multicore \
	          $(BARE_BIN)/programs $(BARE_BIN)/benchmarks

# --- Linux user binaries ---
$(BIN)/programs/%.elf: $(PROG_DIR)/%.c | dirs
	@echo "  CC (SE) $<"
	@$(SE_CC) $(SE_CFLAGS) -O3 $< -o $@

$(BIN)/programs/%.elf: $(PROG_DIR)/%.s | dirs
	@echo "  AS (SE) $<"
	@$(SE_CC) $(SE_CFLAGS) -nostdlib -nostartfiles $< -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/microbenchmarks/%.c | dirs
	@echo "  CC (SE) $<"
	@$(SE_CC) $(SE_CFLAGS) -O0 $< -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/synthetic/%.c | dirs
	@echo "  CC (SE) $<"
	@$(SE_CC) $(SE_CFLAGS) -O3 $< -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/kernels/%.c | dirs
	@echo "  CC (SE) $<"
	@$(SE_CC) $(SE_CFLAGS) -O3 $< -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/complete_prog/%.c | dirs
	@echo "  CC (SE) $<"
	@$(SE_CC) $(SE_CFLAGS) -O3 $< -o $@

# --- Bare-metal libc, once printing through the UART and once through HTIF ---
$(UART_LIB_DIR)/%.o: $(LIB_DIR)/%.c $(LIBC_HEADERS) | dirs
	@echo "  CC (libc) $<"
	@$(CC) $(BARE_CFLAGS) -O3 -c $< -o $@

$(UART_LIB_DIR)/%.o: $(LIB_DIR)/%.s | dirs
	@echo "  AS (libc) $<"
	@$(CC) $(BARE_CFLAGS) -c $< -o $@

$(HTIF_LIB_DIR)/%.o: $(LIB_DIR)/%.c $(LIBC_HEADERS) | dirs
	@echo "  CC (libc, HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -O3 -c $< -o $@

$(HTIF_LIB_DIR)/%.o: $(LIB_DIR)/%.s | dirs
	@echo "  AS (libc, HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -c $< -o $@

# --- Multi-core programs (every hart runs smp_main) ---
$(BIN)/multicore/%.elf: $(BENCH_DIR)/multicore/%.c $(LIB_DIR)/smp.h $(UART_CRT0_SMP) $(UART_LIBC) | dirs
	@echo "  CC (Multicore) $<"
	@$(CC) $(BARE_CFLAGS) -O3 $(BARE_LDFLAGS) $(UART_CRT0_SMP) $(UART_LIBC) $< -o $@

# --- Bare-metal HTIF builds ---
$(BARE_BIN)/programs/%.elf: $(PROG_DIR)/%.c $(HTIF_LIBC) | dirs
	@echo "  CC (HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -O3 $(BARE_LDFLAGS) $(HTIF_LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/microbenchmarks/%.c $(HTIF_LIBC) | dirs
	@echo "  CC (HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -O0 $(BARE_LDFLAGS) $(HTIF_LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/synthetic/%.c $(HTIF_LIBC) | dirs
	@echo "  CC (HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -O3 $(BARE_LDFLAGS) $(HTIF_LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/kernels/%.c $(HTIF_LIBC) | dirs
	@echo "  CC (HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -O3 $(BARE_LDFLAGS) $(HTIF_LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/complete_prog/%.c $(HTIF_LIBC) | dirs
	@echo "  CC (HTIF) $<"
	@$(CC) $(HTIF_CFLAGS) -O3 $(BARE_LDFLAGS) $(HTIF_LIBC) $< -o $@

# --- Linux (download Buildroot + build via tools/boot_linux.py) ---
linux:
	@echo "[Software] Building Linux (download + build in $(LINUX_DIR))..."
	@cd .. && python3 tools/boot_linux.py --no-boot

clean:
	rm -rf $(BUILD_DIR) $(BIN) $(BARE_BIN) build-htif *.img
	rm -rf $(LINUX_DIR)/output $(LINUX_DIR)/buildroot-*

clean-no-linux:
	rm -rf $(BUILD_DIR) $(BIN) $(BARE_BIN) build-htif *.img
