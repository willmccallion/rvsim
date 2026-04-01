# ═══════════════════════════════════════════════════════════════════════════════
#  rvsim — Build, Test, and Run
# ═══════════════════════════════════════════════════════════════════════════════
#  Run from repo root.  make help  for all targets.
#  Override tools:  CARGO=cargo  MATURIN=maturin  PYTHON=python3
# ═══════════════════════════════════════════════════════════════════════════════

SHELL           := $(shell command -v bash)
.DEFAULT_GOAL   := help

# ── Tools ─────────────────────────────────────────────────────────────────────
CARGO           ?= cargo
MATURIN         ?= $(shell [ -f .venv/bin/maturin ] && echo .venv/bin/maturin || echo maturin)
PYTHON          ?= $(shell [ -f .venv/bin/python3 ] && echo .venv/bin/python3 || echo python3)

# Centralized build directory
BUILD_DIR       := target

# Redirect Python byte-code cache to target/
export PYTHONPYCACHEPREFIX := $(BUILD_DIR)/pycache

# ── Colors (only when stdout is a terminal) ───────────────────────────────────
ifneq ($(TERM),)
  GREEN  := \033[32m
  CYAN   := \033[36m
  BOLD   := \033[1m
  RESET  := \033[0m
else
  GREEN  :=
  CYAN   :=
  BOLD   :=
  RESET  :=
endif

# ── Phony ─────────────────────────────────────────────────────────────────────
.PHONY: help build software examples linux python python-wheel
.PHONY: check test test-coverage clippy fmt fmt-check lint prerelease
.PHONY: arch-test
.PHONY: run-example run-linux
.PHONY: profile-build flamegraph
.PHONY: clean clean-rust clean-python clean-software

# ═══════════════════════════════════════════════════════════════════════════════
#  Help
# ═══════════════════════════════════════════════════════════════════════════════
HELP_W := 28
help:
	@printf "\n$(BOLD)rvsim$(RESET) — RISC-V cycle-accurate simulator\n\n"
	@printf "  $(CYAN)Build$(RESET)\n"
	@printf "    %-$(HELP_W)s  Build Python bindings (editable, maturin)\n" "make build"
	@printf "    %-$(HELP_W)s  Install Python bindings (editable, maturin)\n" "make python"
	@printf "    %-$(HELP_W)s  Build distributable Python wheel\n" "make python-wheel"
	@printf "    %-$(HELP_W)s  Build libc and example RISC-V programs\n" "make software"
	@printf "    %-$(HELP_W)s  Build Linux kernel + rootfs (Buildroot)\n" "make linux"
	@printf "\n  $(CYAN)Development$(RESET)\n"
	@printf "    %-$(HELP_W)s  cargo check (all targets)\n" "make check"
	@printf "    %-$(HELP_W)s  Run Rust tests\n" "make test"
	@printf "    %-$(HELP_W)s  Run Rust tests with coverage (llvm-cov)\n" "make test-coverage"
	@printf "    %-$(HELP_W)s  Run clippy linter\n" "make clippy"
	@printf "    %-$(HELP_W)s  Format all code (Rust, Python, C)\n" "make fmt"
	@printf "    %-$(HELP_W)s  Check formatting without modifying\n" "make fmt-check"
	@printf "    %-$(HELP_W)s  fmt-check + clippy\n" "make lint"
	@printf "    %-$(HELP_W)s  Full pre-release check (git+lint+test+versions+build)\n" "make prerelease"
	@printf "    %-$(HELP_W)s  Run riscv-arch-test compliance suite via riscof\n" "make arch-test"
	@printf "\n  $(CYAN)Run$(RESET)\n"
	@printf "    %-$(HELP_W)s  Build and run quicksort benchmark\n" "make run-example"
	@printf "    %-$(HELP_W)s  Boot Linux (requires 'make linux' first)\n" "make run-linux"
	@printf "\n  $(CYAN)Profiling$(RESET)\n"
	@printf "    %-$(HELP_W)s  Build with profiling symbols\n" "make profile-build"
	@printf "    %-$(HELP_W)s  Generate flamegraph (ARGS=…)\n" "make flamegraph"
	@printf "\n  $(CYAN)Housekeeping$(RESET)\n"
	@printf "    %-$(HELP_W)s  Remove all build artifacts\n" "make clean"
	@printf "    %-$(HELP_W)s  Remove Rust artifacts only\n" "make clean-rust"
	@printf "    %-$(HELP_W)s  Remove Python build artifacts\n" "make clean-python"
	@printf "    %-$(HELP_W)s  Remove software artifacts only\n" "make clean-software"
	@printf "\n"

# ═══════════════════════════════════════════════════════════════════════════════
#  Build
# ═══════════════════════════════════════════════════════════════════════════════

build: python

software:
	@printf "$(GREEN)Building libc and example programs…$(RESET)\n"
	$(MAKE) -C software

examples: software

linux:
	@printf "$(GREEN)Building Linux kernel and rootfs…$(RESET)\n"
	$(MAKE) -C software linux

# Install Python bindings in editable/dev mode via maturin
python:
	@printf "$(GREEN)Installing Python bindings (editable)…$(RESET)\n"
	@if [ ! -d .venv ]; then \
		printf "$(GREEN)Creating .venv…$(RESET)\n"; \
		python3 -m venv .venv; \
	fi
	@.venv/bin/pip install --quiet maturin
	.venv/bin/maturin develop --release

# Build a distributable wheel into target/wheels
python-wheel:
	@printf "$(GREEN)Building Python wheel into $(BUILD_DIR)/wheels…$(RESET)\n"
	$(MATURIN) build --release --out $(BUILD_DIR)/wheels

# ═══════════════════════════════════════════════════════════════════════════════
#  Development
# ═══════════════════════════════════════════════════════════════════════════════

check:
	@printf "$(GREEN)Running cargo check…$(RESET)\n"
	$(CARGO) check --workspace --all-targets

test:
	@printf "$(GREEN)Running Rust tests…$(RESET)\n"
	$(CARGO) test --workspace

test-coverage:
	@printf "$(GREEN)Running cargo llvm-cov…$(RESET)\n"
	@command -v cargo-llvm-cov >/dev/null 2>&1 || { \
		printf "$(BOLD)Error: cargo-llvm-cov not installed.$(RESET)\n"; \
		printf "Install with: $(CYAN)cargo install cargo-llvm-cov$(RESET)\n"; \
		exit 1; \
	}
	$(CARGO) llvm-cov --workspace --exclude rvsim-bindings

clippy:
	@printf "$(GREEN)Running clippy…$(RESET)\n"
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	@printf "$(GREEN)Formatting Rust code…$(RESET)\n"
	$(CARGO) fmt --all
	@printf "$(GREEN)Formatting Python code…$(RESET)\n"
	$(PYTHON) -m ruff format rvsim/*.py

fmt-check:
	@printf "$(GREEN)Checking Rust formatting…$(RESET)\n"
	$(CARGO) fmt --all -- --check
	@printf "$(GREEN)Checking Python formatting…$(RESET)\n"
	$(PYTHON) -m ruff format --check rvsim/*.py

lint: fmt-check clippy

arch-test:
	@printf "$(GREEN)Running riscv-arch-test compliance suite via riscof…$(RESET)\n"
	@if [ ! -d testing/riscof/riscv-arch-test ]; then \
		printf "$(GREEN)Cloning riscv-arch-test suite…$(RESET)\n"; \
		.venv/bin/riscof arch-test --clone --dir testing/riscof/riscv-arch-test; \
	fi
	cd testing/riscof && ../../.venv/bin/riscof run --no-browser \
		--config config.ini \
		--suite riscv-arch-test/riscv-test-suite/ \
		--env riscv-arch-test/riscv-test-suite/env

prerelease:
	@tools/prerelease

# ═══════════════════════════════════════════════════════════════════════════════
#  Run
# ═══════════════════════════════════════════════════════════════════════════════

run-example: software
	@printf "$(GREEN)Running quicksort benchmark…$(RESET)\n"
	.venv/bin/rvsim -f software/bin/benchmarks/qsort.elf

run-linux:
	@printf "$(GREEN)Booting Linux…$(RESET)\n"
	.venv/bin/rvsim --script scripts/setup/boot_linux.py

# ═══════════════════════════════════════════════════════════════════════════════
#  Profiling
# ═══════════════════════════════════════════════════════════════════════════════

profile-build:
	@printf "$(GREEN)Building with profiling symbols…$(RESET)\n"
	.venv/bin/maturin develop --profile profiling

flamegraph:
	@printf "$(GREEN)Recording flamegraph…$(RESET)\n"
	$$HOME/.cargo/bin/flamegraph -o flamegraph.svg -F 99 -- .venv/bin/rvsim $(ARGS)

# ═══════════════════════════════════════════════════════════════════════════════
#  Housekeeping
# ═══════════════════════════════════════════════════════════════════════════════

clean:
	@printf "$(GREEN)Cleaning all artifacts (removing $(BUILD_DIR))…$(RESET)\n"
	rm -rf $(BUILD_DIR)
	@$(MAKE) -C software clean

clean-python:
	@printf "$(GREEN)Removing Python build artifacts…$(RESET)\n"
	rm -rf $(BUILD_DIR)/wheels $(BUILD_DIR)/pycache
	find rvsim -name '*.so' -delete 2>/dev/null || true
	rm -rf build *.egg-info

clean-rust:
	@printf "$(GREEN)Removing Rust build artifacts…$(RESET)\n"
	$(CARGO) clean

clean-software:
	@printf "$(GREEN)Removing software build artifacts…$(RESET)\n"
	@if [ -d software/linux/buildroot-2024.08/output ]; then \
		printf "Remove Linux build output? [y/N] "; \
		read answer; \
		case "$$answer" in \
			[yY]) $(MAKE) -C software clean ;; \
			*) $(MAKE) -C software clean-no-linux ;; \
		esac; \
	else \
		$(MAKE) -C software clean; \
	fi
