# Builds the bundled programs:
#   bin/programs, bin/benchmarks   static Linux user binaries, run under SE mode
#   bin/multicore                  bare-metal; every hart runs smp_main
#   bin-htif/programs, benchmarks  bare-metal builds of the same sources
# Bare-metal programs print and exit through HTIF, as on spike and the RTL
# simulators.
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

LIBC_DIR = $(BUILD_DIR)/libc
LIBC_HEADERS = $(LIB_DIR)/stdio.h $(LIB_DIR)/stdlib.h $(LIB_DIR)/htif.h $(LIB_DIR)/fdt.h
LIBC = $(LIBC_DIR)/stdio.o $(LIBC_DIR)/stdlib.o $(LIBC_DIR)/htif.o $(LIBC_DIR)/fdt.o
CRT0 = $(LIBC_DIR)/crt0_htif.o
CRT0_SMP = $(LIBC_DIR)/crt0_smp.o

.PHONY: all dirs linux clean clean-no-linux

all: $(SE_ELFS) $(MULTICORE_ELFS) $(HTIF_ELFS)
	@echo "  Built $(words $(SE_ELFS) $(MULTICORE_ELFS) $(HTIF_ELFS)) ELFs"
	@echo "  SE mode:    $(BIN)/programs/ $(BIN)/benchmarks/"
	@echo "  Bare-metal: $(BIN)/multicore/ $(BARE_BIN)/programs/ $(BARE_BIN)/benchmarks/"

dirs:
	@mkdir -p $(LIBC_DIR) \
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

# --- Bare-metal libc ---
$(LIBC_DIR)/%.o: $(LIB_DIR)/%.c $(LIBC_HEADERS) | dirs
	@echo "  CC (libc) $<"
	@$(CC) $(BARE_CFLAGS) -O3 -c $< -o $@

$(LIBC_DIR)/%.o: $(LIB_DIR)/%.s | dirs
	@echo "  AS (libc) $<"
	@$(CC) $(BARE_CFLAGS) -c $< -o $@

# --- Multi-core programs (every hart runs smp_main) ---
$(BIN)/multicore/%.elf: $(BENCH_DIR)/multicore/%.c $(LIB_DIR)/smp.h $(CRT0_SMP) $(LIBC) | dirs
	@echo "  CC (Multicore) $<"
	@$(CC) $(BARE_CFLAGS) -O3 $(BARE_LDFLAGS) $(CRT0_SMP) $(LIBC) $< -o $@

# --- Bare-metal builds of the programs and benchmarks ---
$(BARE_BIN)/programs/%.elf: $(PROG_DIR)/%.c $(CRT0) $(LIBC) | dirs
	@echo "  CC (bare) $<"
	@$(CC) $(BARE_CFLAGS) -O3 $(BARE_LDFLAGS) $(CRT0) $(LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/microbenchmarks/%.c $(CRT0) $(LIBC) | dirs
	@echo "  CC (bare) $<"
	@$(CC) $(BARE_CFLAGS) -O0 $(BARE_LDFLAGS) $(CRT0) $(LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/synthetic/%.c $(CRT0) $(LIBC) | dirs
	@echo "  CC (bare) $<"
	@$(CC) $(BARE_CFLAGS) -O3 $(BARE_LDFLAGS) $(CRT0) $(LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/kernels/%.c $(CRT0) $(LIBC) | dirs
	@echo "  CC (bare) $<"
	@$(CC) $(BARE_CFLAGS) -O3 $(BARE_LDFLAGS) $(CRT0) $(LIBC) $< -o $@

$(BARE_BIN)/benchmarks/%.elf: $(BENCH_DIR)/complete_prog/%.c $(CRT0) $(LIBC) | dirs
	@echo "  CC (bare) $<"
	@$(CC) $(BARE_CFLAGS) -O3 $(BARE_LDFLAGS) $(CRT0) $(LIBC) $< -o $@

# --- Linux (download Buildroot + build via tools/boot_linux.py) ---
linux:
	@echo "[Software] Building Linux (download + build in $(LINUX_DIR))..."
	@cd .. && python3 tools/boot_linux.py --no-boot

clean:
	rm -rf $(BUILD_DIR) $(BIN) $(BARE_BIN) build-htif *.img
	rm -rf $(LINUX_DIR)/output $(LINUX_DIR)/buildroot-*

clean-no-linux:
	rm -rf $(BUILD_DIR) $(BIN) $(BARE_BIN) build-htif *.img
