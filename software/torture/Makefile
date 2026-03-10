# Torture test build system
# Generates, assembles, and runs OoO pipeline torture tests

CC       = riscv64-elf-gcc
OBJDUMP  = riscv64-elf-objdump

RISCV_TESTS = ../riscv-tests
ENV_DIR     = $(RISCV_TESTS)/env
ISA_MACROS  = $(RISCV_TESTS)/isa/macros/scalar
LINK_SCRIPT = $(ENV_DIR)/p/link.ld

CFLAGS = -march=rv64gc -mabi=lp64d -static -mcmodel=medany \
         -fvisibility=hidden -nostdlib -nostartfiles \
         -I$(ENV_DIR)/p -I$(ENV_DIR) -I$(ISA_MACROS) \
         -T$(LINK_SCRIPT)

GEN_DIR   = generated
BUILD_DIR = build
FAIL_DIR  = failures

SEED    ?= 0
COUNT   ?= 100
LENGTH  ?= 500
MEM_PCT ?= 30
BR_PCT  ?= 15

SRCS = $(sort $(wildcard $(GEN_DIR)/*.S))
ELFS = $(patsubst $(GEN_DIR)/%.S,$(BUILD_DIR)/%,$(SRCS))

.PHONY: all generate build run clean help

all: generate build

help:
	@echo "Torture test targets:"
	@echo "  make generate          Generate .S files (SEED=$(SEED) COUNT=$(COUNT) LENGTH=$(LENGTH))"
	@echo "  make build             Assemble generated .S -> ELF"
	@echo "  make run               Generate + build + run against Spike & rvsim"
	@echo "  make clean             Remove generated files and build artifacts"
	@echo ""
	@echo "Variables:"
	@echo "  SEED=0       Starting seed"
	@echo "  COUNT=100    Number of tests"
	@echo "  LENGTH=500   Instructions per test"
	@echo "  MEM_PCT=30   Memory operation percentage"
	@echo "  BR_PCT=15    Branch block percentage"

generate:
	@mkdir -p $(GEN_DIR)
	python3 generate.py --seed $(SEED) --count $(COUNT) --length $(LENGTH) \
	    --mem-pct $(MEM_PCT) --branch-pct $(BR_PCT) --outdir $(GEN_DIR)

$(BUILD_DIR)/%: $(GEN_DIR)/%.S
	@mkdir -p $(BUILD_DIR)
	$(CC) $(CFLAGS) -o $@ $<

build: $(ELFS)
	@echo "Built $(words $(ELFS)) torture test ELFs"

run:
	@mkdir -p $(BUILD_DIR) $(GEN_DIR)
	cd ../.. && python3 software/torture/run.py \
	    --seed $(SEED) --count $(COUNT) --length $(LENGTH) \
	    --mem-pct $(MEM_PCT) --branch-pct $(BR_PCT) \
	    --keep-failing

clean:
	rm -rf $(GEN_DIR) $(BUILD_DIR) $(FAIL_DIR)
