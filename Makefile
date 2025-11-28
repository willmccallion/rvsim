TARGET = riscv64-elf
CC = $(TARGET)-gcc
LD = $(TARGET)-ld
OBJCOPY = $(TARGET)-objcopy

# --- Directory Structure ---
BUILD_DIR = build
BIN_DIR   = bin
# New paths based on the restructure
LIB_DIR   = software/libc
SRC_DIR   = software/user

# --- Flags ---
# -O2: Enable optimizations (requires stack alignment fix in kernel.s)
# -ffreestanding: Don't assume standard library exists
# -I$(LIB_DIR): Include headers from software/libc
CFLAGS = -march=rv64g -mabi=lp64 -mcmodel=medany -ffreestanding -nostdlib -g -O0 -I$(LIB_DIR)

# --- Shared Library Sources ---
CRT0_SRC   = $(LIB_DIR)/crt0.s
STDIO_SRC  = $(LIB_DIR)/stdio.c
STDLIB_SRC = $(LIB_DIR)/stdlib.c
LINKER_SCR = $(LIB_DIR)/user.ld

# --- Shared Library Objects ---
CRT0_OBJ   = $(BUILD_DIR)/libc/crt0.o
STDIO_OBJ  = $(BUILD_DIR)/libc/stdio.o
STDLIB_OBJ = $(BUILD_DIR)/libc/stdlib.o

# --- User C Programs ---
USER_C_SRCS = $(wildcard $(SRC_DIR)/*.c)
# Map software/user/xxx.c -> build/user/xxx.o
USER_C_OBJS = $(patsubst $(SRC_DIR)/%.c, $(BUILD_DIR)/user/%.o, $(USER_C_SRCS))
# Map build/user/xxx.o -> bin/xxx.bin
USER_C_BINS = $(patsubst $(BUILD_DIR)/user/%.o, $(BIN_DIR)/%.bin, $(USER_C_OBJS))

# --- User Assembly Programs ---
USER_S_SRCS = $(wildcard $(SRC_DIR)/*.s)
USER_S_OBJS = $(patsubst $(SRC_DIR)/%.s, $(BUILD_DIR)/user/%.o, $(USER_S_SRCS))
USER_S_BINS = $(patsubst $(BUILD_DIR)/user/%.o, $(BIN_DIR)/%.bin, $(USER_S_OBJS))

.PHONY: all clean run dirs

all: dirs $(USER_C_BINS) $(USER_S_BINS)

dirs:
	@mkdir -p $(BUILD_DIR)/libc
	@mkdir -p $(BUILD_DIR)/user
	@mkdir -p $(BIN_DIR)

# --- Library Compilation ---
$(CRT0_OBJ): $(CRT0_SRC)
	@echo "  AS (Lib) $<"
	@$(CC) $(CFLAGS) -c $< -o $@

$(STDIO_OBJ): $(STDIO_SRC)
	@echo "  CC (Lib) $<"
	@$(CC) $(CFLAGS) -c $< -o $@

$(STDLIB_OBJ): $(STDLIB_SRC)
	@echo "  CC (Lib) $<"
	@$(CC) $(CFLAGS) -c $< -o $@

# --- User C Linking ---
# Links with CRT0, Stdio, and Stdlib
$(USER_C_BINS): $(BIN_DIR)/%.bin: $(BUILD_DIR)/user/%.o $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ)
	@echo "  LD (C)   $@"
	@$(LD) -T $(LINKER_SCR) $(CRT0_OBJ) $(STDIO_OBJ) $(STDLIB_OBJ) $< -o $(BUILD_DIR)/user/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/user/$*.elf $@

$(BUILD_DIR)/user/%.o: $(SRC_DIR)/%.c
	@echo "  CC       $<"
	@$(CC) $(CFLAGS) -c $< -o $@

# --- User Assembly Linking ---
# Standalone (No CRT0, No Libs)
$(USER_S_BINS): $(BIN_DIR)/%.bin: $(BUILD_DIR)/user/%.o
	@echo "  LD (Asm) $@"
	@$(LD) -T $(LINKER_SCR) $< -o $(BUILD_DIR)/user/$*.elf
	@$(OBJCOPY) -O binary $(BUILD_DIR)/user/$*.elf $@

$(BUILD_DIR)/user/%.o: $(SRC_DIR)/%.s
	@echo "  AS       $<"
	@$(CC) $(CFLAGS) -c $< -o $@

run: all
	@if [ -z "$(BIN)" ]; then \
		echo "Usage: make run BIN=<program.bin>"; \
	else \
		echo "Running $(BIN)..."; \
		cargo run --release -- bin/$(BIN); \
	fi

clean:
	@echo "Cleaning..."
	@rm -rf $(BUILD_DIR) $(BIN_DIR)
