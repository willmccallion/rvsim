# RISC-V Software Build System
# Builds libc and example programs for the simulator
# Supports RV64GC (IMAFD + C) with double-precision hard-float ABI

TARGET ?= riscv64-elf
CC = $(TARGET)-gcc
LD = $(TARGET)-ld

# Directories
BUILD_DIR ?= build
BIN ?= bin
# IO=uart prints through the UART and exits with an ecall; IO=htif prints
# and exits through HTIF, as on spike and the RTL harness.
IO ?= uart
LIB_DIR   = libc
EXAM_DIR  = ../examples
LINUX_DIR = linux

# Base Flags
# -march=rv64gcv: RV64I + M (multiply) + A (atomics) + F (single-float) + D (double-float) + C (compressed) + V (vector)
# -mabi=lp64d: LP64 with double-precision hard-float calling convention
CFLAGS = -march=rv64gcv -mabi=lp64d -mcmodel=medany -ffreestanding -nostdlib -g
INCLUDES = -I$(LIB_DIR)

# Specific Optimization Levels
ifeq ($(IO),htif)
CFLAGS += -DRVSIM_IO_HTIF
endif
USER_CFLAGS  = $(CFLAGS) $(INCLUDES) -O3
MICRO_CFLAGS = $(CFLAGS) $(INCLUDES) -O0
APP_CFLAGS   = $(CFLAGS) $(INCLUDES) -O3

# Objects
CRT0_OBJ   = $(BUILD_DIR)/libc/crt0.o
HTIF_OBJ   = $(BUILD_DIR)/libc/htif.o
LIBC_IO_OBJS =
CRT0_SMP_OBJ = $(BUILD_DIR)/libc/crt0_smp.o
STDIO_OBJ  = $(BUILD_DIR)/libc/stdio.o
STDLIB_OBJ = $(BUILD_DIR)/libc/stdlib.o
LINKER_SCR = $(LIB_DIR)/user.ld
ifeq ($(IO),htif)
CRT0_OBJ = $(BUILD_DIR)/libc/crt0_htif.o
LIBC_IO_OBJS = $(HTIF_OBJ)
endif

# --- Sources from ../examples ---
PROG_DIR = $(EXAM_DIR)/programs
BENCH_DIR = $(EXAM_DIR)/benchmarks

PROG_C_SRCS = $(wildcard $(PROG_DIR)/*.c)
PROG_C_ELFS = $(patsubst $(PROG_DIR)/%.c, $(BIN)/programs/%.elf, $(PROG_C_SRCS))
PROG_S_SRCS = $(wildcard $(PROG_DIR)/*.s)
PROG_S_ELFS = $(patsubst $(PROG_DIR)/%.s, $(BIN)/programs/%.elf, $(PROG_S_SRCS))

BENCH_MICRO_SRCS = $(wildcard $(BENCH_DIR)/microbenchmarks/*.c)
BENCH_SYNTH_SRCS = $(wildcard $(BENCH_DIR)/synthetic/*.c)
BENCH_KERN_SRCS  = $(wildcard $(BENCH_DIR)/kernels/*.c)
BENCH_PROG_SRCS  = $(wildcard $(BENCH_DIR)/complete_prog/*.c)

MULTICORE_SRCS = $(wildcard $(BENCH_DIR)/multicore/*.c)
MULTICORE_ELFS = $(patsubst $(BENCH_DIR)/multicore/%.c, $(BIN)/multicore/%.elf, $(MULTICORE_SRCS))

BENCH_ELFS = \
	$(patsubst $(BENCH_DIR)/microbenchmarks/%.c, $(BIN)/benchmarks/%.elf, $(BENCH_MICRO_SRCS)) \
	$(patsubst $(BENCH_DIR)/synthetic/%.c, $(BIN)/benchmarks/%.elf, $(BENCH_SYNTH_SRCS)) \
	$(patsubst $(BENCH_DIR)/kernels/%.c, $(BIN)/benchmarks/%.elf, $(BENCH_KERN_SRCS)) \
	$(patsubst $(BENCH_DIR)/complete_prog/%.c, $(BIN)/benchmarks/%.elf, $(BENCH_PROG_SRCS))

.PHONY: all clean clean-no-linux dirs disk linux

all: dirs disk

dirs:
	@mkdir -p $(BUILD_DIR)/libc $(BUILD_DIR)/programs \
	          $(BUILD_DIR)/benchmarks $(BUILD_DIR)/multicore \
	          $(BIN)/programs $(BIN)/benchmarks $(BIN)/multicore

# --- Libc ---
$(BUILD_DIR)/libc/crt0.o: $(LIB_DIR)/crt0.s
	@echo "  AS (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(BUILD_DIR)/libc/crt0_htif.o: $(LIB_DIR)/crt0_htif.s
	@echo "  AS (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(HTIF_OBJ): $(LIB_DIR)/htif.c $(LIB_DIR)/htif.h
	@echo "  CC (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(CRT0_SMP_OBJ): $(LIB_DIR)/crt0_smp.s
	@echo "  AS (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(STDIO_OBJ): $(LIB_DIR)/stdio.c $(LIB_DIR)/htif.h
	@echo "  CC (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

$(STDLIB_OBJ): $(LIB_DIR)/stdlib.c
	@echo "  CC (Lib) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $@

# --- Programs ---
$(BIN)/programs/%.elf: $(PROG_DIR)/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS)
	@echo "  CC (Prog) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $(BUILD_DIR)/programs/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) $(BUILD_DIR)/programs/$*.o -o $@

$(BIN)/programs/%.elf: $(PROG_DIR)/%.s
	@echo "  AS (Prog) $<"
	@$(CC) $(USER_CFLAGS) -c $< -o $(BUILD_DIR)/programs/$*.o
	@$(LD) -T $(LINKER_SCR) $(BUILD_DIR)/programs/$*.o -o $@

# --- Benchmarks ---
$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/microbenchmarks/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS)
	@echo "  CC (Bench/Micro) $<"
	@$(CC) $(MICRO_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) $(BUILD_DIR)/benchmarks/$*.o -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/synthetic/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS)
	@echo "  CC (Bench/Synth) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) $(BUILD_DIR)/benchmarks/$*.o -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/kernels/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS)
	@echo "  CC (Bench/Kern) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) $(BUILD_DIR)/benchmarks/$*.o -o $@

$(BIN)/benchmarks/%.elf: $(BENCH_DIR)/complete_prog/%.c $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS)
	@echo "  CC (Bench/Prog) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/benchmarks/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) $(BUILD_DIR)/benchmarks/$*.o -o $@

# --- Multi-core programs (every hart runs smp_main) ---
$(BIN)/multicore/%.elf: $(BENCH_DIR)/multicore/%.c $(LIB_DIR)/smp.h $(CRT0_SMP_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS)
	@echo "  CC (Multicore) $<"
	@$(CC) $(APP_CFLAGS) -c $< -o $(BUILD_DIR)/multicore/$*.o
	@$(LD) -T $(LINKER_SCR) $(CRT0_SMP_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) $(BUILD_DIR)/multicore/$*.o -o $@

# --- CoreMark ---
# Download source first:
#   git clone https://github.com/eembc/coremark /tmp/coremark
#   cp /tmp/coremark/core_{list_join,matrix,state,util,main}.c examples/benchmarks/coremark/
#   cp /tmp/coremark/coremark.h                                 examples/benchmarks/coremark/
# Then: make coremark

COREMARK_DIR  = $(EXAM_DIR)/benchmarks/coremark
COREMARK_SRCS = \
	$(COREMARK_DIR)/core_list_join.c \
	$(COREMARK_DIR)/core_matrix.c    \
	$(COREMARK_DIR)/core_state.c     \
	$(COREMARK_DIR)/core_util.c      \
	$(COREMARK_DIR)/core_main.c      \
	$(COREMARK_DIR)/core_portme.c
COREMARK_ELF  = $(BIN)/benchmarks/coremark.elf

.PHONY: coremark
coremark: dirs $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@if [ ! -f $(COREMARK_DIR)/coremark.h ]; then \
		echo "ERROR: CoreMark sources missing.  Run:"; \
		echo "  git clone https://github.com/eembc/coremark /tmp/coremark"; \
		echo "  cp /tmp/coremark/core_{list_join,matrix,state,util,main}.c $(COREMARK_DIR)/"; \
		echo "  cp /tmp/coremark/coremark.h $(COREMARK_DIR)/"; \
		exit 1; \
	fi
	@echo "  CC (CoreMark)"
	@$(CC) $(APP_CFLAGS) \
		-DPERFORMANCE_RUN=1 \
		-I$(COREMARK_DIR) \
		-I$(LIB_DIR) \
		$(COREMARK_SRCS) \
		$(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $(LIBC_IO_OBJS) \
		-T $(LINKER_SCR) \
		-o $(COREMARK_ELF)
	@echo "  -> $(COREMARK_ELF)"

# --- Linux (download Buildroot + build via tools/boot_linux.py) ---
linux:
	@echo "[Software] Building Linux (download + build in $(LINUX_DIR))..."
	@cd .. && python3 tools/boot_linux.py --no-boot

# --- Disk Image ---
disk: $(PROG_C_ELFS) $(PROG_S_ELFS) $(BENCH_ELFS) $(MULTICORE_ELFS)
	@echo "  Built $(words $(PROG_C_ELFS) $(PROG_S_ELFS) $(BENCH_ELFS) $(MULTICORE_ELFS)) ELFs"
	@echo "  Programs: $(BIN)/programs/"
	@echo "  Benchmarks: $(BIN)/benchmarks/"

clean:
	rm -rf build bin build-htif bin-htif *.img
	rm -rf $(LINUX_DIR)/output $(LINUX_DIR)/buildroot-*

clean-no-linux:
	rm -rf build bin build-htif bin-htif *.img
