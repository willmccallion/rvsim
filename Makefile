# RISC-V System Simulator - Top-level Makefile
# Build the simulator (Rust) and example software (C/Assembly)

.PHONY: all help simulator software examples linux clean check test run-example run-linux

# Default target
all: simulator software

# ============================================================================
# Help Target
# ============================================================================
help:
	@echo "RISC-V System Simulator - Build Targets"
	@echo "========================================"
	@echo ""
	@echo "Main Targets:"
	@echo "  all              Build simulator and software (default)"
	@echo "  simulator        Build Rust simulator (release binary at target/release/sim)"
	@echo "  software         Build libc and example programs"
	@echo "  examples         Alias for 'software'"
	@echo "  linux            Download and build Linux kernel + rootfs"
	@echo ""
	@echo "Development:"
	@echo "  check            Run cargo check on all Rust crates"
	@echo "  test             Run Rust tests (cargo test)"
	@echo "  clippy           Run cargo clippy linter"
	@echo "  fmt              Format Rust code with cargo fmt"
	@echo ""
	@echo "Running:"
	@echo "  run-example      Build and run quicksort benchmark"
	@echo "  run-linux        Build and boot Linux (requires linux target first)"
	@echo ""
	@echo "Maintenance:"
	@echo "  clean            Remove all build artifacts"
	@echo "  clean-rust       Remove Rust build artifacts only"
	@echo "  clean-software   Remove software build artifacts only"
	@echo ""
	@echo "Python:"
	@echo "  python-dev       Install Python package in development mode"
	@echo "  python-test      Run Python tests/scripts"
	@echo ""

# ============================================================================
# Build Targets
# ============================================================================

# Build Rust simulator (creates CLI tool at target/release/sim)
simulator:
	@echo "[Simulator] Building Rust simulator (release)..."
	@cargo build --release

# Build example software (benchmarks and programs)
software:
	@echo "[Software] Building libc and examples..."
	@$(MAKE) -C software

examples: software

# Download and build Linux kernel + rootfs via Buildroot
linux:
	@echo "[Linux] Building Linux kernel and rootfs..."
	@$(MAKE) -C software linux

# ============================================================================
# Development Targets
# ============================================================================

# Check Rust code without building
check:
	@echo "[Check] Running cargo check..."
	@cargo check --workspace --all-targets

# Run Rust tests
test:
	@echo "[Test] Running cargo test..."
	@cargo test --workspace

# Run clippy linter
clippy:
	@echo "[Clippy] Running cargo clippy..."
	@cargo clippy --workspace --all-targets -- -D warnings

# Format Rust code
fmt:
	@echo "[Format] Running cargo fmt..."
	@cargo fmt --all

# ============================================================================
# Python Development
# ============================================================================

# Install Python package in development mode
python-dev:
	@echo "[Python] Installing riscv_sim in development mode..."
	@pip install -e .

# Run Python benchmark scripts
python-test:
	@echo "[Python] Running benchmark scripts..."
	@./target/release/sim script scripts/benchmarks/tests/smoke_test.py

# ============================================================================
# Running Targets
# ============================================================================

# Quick test: run quicksort benchmark
run-example: simulator software
	@echo "[Run] Running quicksort benchmark..."
	@./target/release/sim run -f software/bin/benchmarks/qsort.bin

# Boot Linux (requires linux target to be built first)
run-linux: simulator
	@echo "[Run] Booting Linux..."
	@./target/release/sim script scripts/setup/boot_linux.py

# ============================================================================
# Cleaning
# ============================================================================

clean: clean-rust clean-software
	@echo "[Clean] All build artifacts removed"

clean-rust:
	@echo "[Clean] Removing Rust build artifacts..."
	@cargo clean

clean-software:
	@echo "[Clean] Removing software build artifacts..."
	@$(MAKE) -C software clean
