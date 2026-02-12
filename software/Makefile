# RISC-V Software Build System
# Builds libc and example programs for the simulator
# Supports RV64GC (IMAFD + C) with double-precision hard-float ABI

TARGET = riscv64-elf
CC = $(TARGET)-gcc
LD = $(TARGET)-ld
OBJCOPY = $(TARGET)-objcopy

# Directories
BUILD_DIR = build
LIB_DIR   = libc
EXAM_DIR  = ../examples
LINUX_DIR = linux

# Base Flags
# -march=rv64gc: RV64I + M (multiply) + A (atomics) + F (single-float) + D (double-float) + C (compressed)
# -mabi=lp64d: LP64 with double-precision hard-float calling convention
CFLAGS = -march=rv64gc -mabi=lp64d -mcmodel=medany -ffreestanding -nostdlib -g
INCLUDES = -I$(LIB_DIR)

# Specific Optimization Levels
USER_CFLAGS  = $(CFLAGS) $(INCLUDES) -O3
MICRO_CFLAGS = $(CFLAGS) $(INCLUDES) -O0
APP_CFLAGS   = $(CFLAGS) $(INCLUDES) -O3

# Objects
CRT0_OBJ   = $(BUILD_DIR)/libc/crt0.o
STDIO_OBJ  = $(BUILD_DIR)/libc/stdio.o
STDLIB_OBJ = $(BUILD_DIR)/libc/stdlib.o
LINKER_SCR = $(LIB_DIR)/user.ld

# --- Sources from ../examples ---
PROG_DIR = $(EXAM_DIR)/programs
BENCH_DIR = $(EXAM_DIR)/benchmarks

PROG_C_SRCS = $(wildcard $(PROG_DIR)/*.c)
PROG_C_BINS = $(patsubst $(PROG_DIR)/%.c, bin/programs/%.bin, $(PROG_C_SRCS))
PROG_S_SRCS = $(wildcard $(PROG_DIR)/*.s)
PROG_S_BINS = $(patsubst $(PROG_DIR)/%.s, bin/programs/%.bin, $(PROG_S_SRCS))

BENCH_MICRO_SRCS = $(wildcard $(BENCH_DIR)/microbenchmarks/*.c)
BENCH_SYNTH_SRCS = $(wildcard $(BENCH_DIR)/synthetic/*.c)
BENCH_KERN_SRCS  = $(wildcard $(BENCH_DIR)/kernels/*.c)
BENCH_PROG_SRCS  = $(wildcard $(BENCH_DIR)/complete_prog/*.c)

BENCH_BINS = \
	$(patsubst $(BENCH_DIR)/microbenchmarks/%.c, bin/benchmarks/%.bin, $(BENCH_MICRO_SRCS)) \
	$(patsubst $(BENCH_DIR)/synthetic/%.c, bin/benchmarks/%.bin, $(BENCH_SYNTH_SRCS)) \
	$(patsubst $(BENCH_DIR)/kernels/%.c, bin/benchmarks/%.bin, $(BENCH_KERN_SRCS)) \
	$(patsubst $(BENCH_DIR)/complete_prog/%.c, bin/benchmarks/%.bin, $(BENCH_PROG_SRCS))

.PHONY: all clean dirs disk linux

all: dirs disk

dirs:
	@mkdir -p $(BUILD_DIR)/libc $(BUILD_DIR)/programs \
	          $(BUILD_DIR)/benchmarks \
	          bin/programs bin/benchmarks

# --- Libc ---
$(CRT0_OBJ): $(LIB_DIR)/crt0.s
	@echo "  AS (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(STDIO_OBJ): $(LIB_DIR)/stdio.c
	@echo "  CC (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(STDLIB_OBJ): $(LIB_DIR)/stdlib.c
	@echo "  CC (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

# --- Programs ---
bin/programs/%.bin: $(PROG_DIR)/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@echo "  CC (Prog) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $(BUILD_DIR)/programs/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(BUILD_DIR)/programs/$*.o -o $(BUILD_DIR)/programs/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/programs/$*.elf $@

bin/programs/%.bin: $(PROG_DIR)/%.s
	@echo "  AS (Prog) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $(BUILD_DIR)/programs/$*.o
	@$(LD) -T $(LINKER_SCR) $(BUILD_DIR)/programs/$*.o -o $(BUILD_DIR)/programs/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/programs/$*.elf $@

# --- Benchmarks ---
bin/benchmarks/%.bin: $(BENCH_DIR)/microbenchmarks/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@echo "  CC (Bench/Micro) $<"
	@$(CC) $(MICRO_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(BUILD_DIR)/benchmarks/$*.o -o $(BUILD_DIR)/benchmarks/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/benchmarks/$*.elf $@

bin/benchmarks/%.bin: $(BENCH_DIR)/synthetic/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@echo "  CC (Bench/Synth) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(BUILD_DIR)/benchmarks/$*.o -o $(BUILD_DIR)/benchmarks/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/benchmarks/$*.elf $@

bin/benchmarks/%.bin: $(BENCH_DIR)/kernels/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@echo "  CC (Bench/Kern) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(BUILD_DIR)/benchmarks/$*.o -o $(BUILD_DIR)/benchmarks/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/benchmarks/$*.elf $@

bin/benchmarks/%.bin: $(BENCH_DIR)/complete_prog/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@echo "  CC (Bench/Prog) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(BUILD_DIR)/benchmarks/$*.o -o $(BUILD_DIR)/benchmarks/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/benchmarks/$*.elf $@

# --- Linux (download Buildroot + build via scripts/setup/boot_linux.py) ---
linux:
	@echo "[Software] Building Linux (download + build in $(LINUX_DIR))..."
	@cd .. && python3 scripts/setup/boot_linux.py --no-boot

# --- Disk Image ---
disk: $(PROG_C_BINS) $(PROG_S_BINS) $(BENCH_BINS)
	@echo "  Built $(words $(PROG_C_BINS) $(PROG_S_BINS) $(BENCH_BINS)) binaries"
	@echo "  Programs: bin/programs/"
	@echo "  Benchmarks: bin/benchmarks/"

clean:
	rm -rf build bin *.img
	rm -rf $(LINUX_DIR)/output $(LINUX_DIR)/buildroot-*
