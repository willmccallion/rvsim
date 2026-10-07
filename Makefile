# Run from the repo root; `make help` lists the targets.
# Override tools: CARGO=cargo MATURIN=maturin PYTHON=python3

SHELL           := $(shell command -v bash)
.DEFAULT_GOAL   := help

CARGO           ?= cargo
MATURIN         ?= $(shell [ -f .venv/bin/maturin ] && echo .venv/bin/maturin || echo maturin)
PYTHON          ?= $(shell [ -f .venv/bin/python3 ] && echo .venv/bin/python3 || echo python3)

# Centralized build directory
BUILD_DIR       := target
TEST_BUILDS     := tests/builds

# Redirect Python byte-code cache to target/
export PYTHONPYCACHEPREFIX := $(BUILD_DIR)/pycache

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

.PHONY: help build software examples linux python python-wheel
.PHONY: check test test-python test-coverage clippy fmt fmt-check lint prerelease
.PHONY: compare-gem5
.PHONY: vector-test vector-test-build vector-smoke-build vector-test-smoke vector-test-multi
.PHONY: riscv-tests riscv-tests-build
.PHONY: test-all test-all-smoke conformance-smoke clean-tests
.PHONY: run-example run-linux
.PHONY: profile-build flamegraph
.PHONY: clean clean-rust clean-python clean-software

HELP_W := 28
help:
	@printf "\n$(BOLD)rvsim$(RESET) — RISC-V cycle-level simulator\n\n"
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
	@printf "    %-$(HELP_W)s  Full pre-release check (PRERELEASE_ARGS=--skip-linux skips the boot)\n" "make prerelease"
	@printf "    %-$(HELP_W)s  Build riscv-tests ELFs (one-time)\n" "make riscv-tests-build"
	@printf "    %-$(HELP_W)s  Run riscv-tests across all PIPELINES\n" "make riscv-tests"
	@printf "    %-$(HELP_W)s  Build chipsalliance RVV test ELFs (one-time)\n" "make vector-test-build"
	@printf "    %-$(HELP_W)s  Run RVV cosim suite (rvsim vs spike)\n" "make vector-test"
	@printf "    %-$(HELP_W)s  Smoke RVV suite (a sample of every instruction class)\n" "make vector-test-smoke"
	@printf "    %-$(HELP_W)s  Run RVV cosim across all PIPELINES (slow)\n" "make vector-test-multi"
	@printf "    %-$(HELP_W)s  Run EVERY suite x EVERY PIPELINES (very slow)\n" "make test-all"
	@printf "    %-$(HELP_W)s  Rust, Python and conformance smoke (CI's gate)\n" "make test-all-smoke"
	@printf "    %-$(HELP_W)s  Conformance smoke only (riscv, vector, multicore)\n" "make conformance-smoke"
	@printf "    %-$(HELP_W)s  Wipe tests/builds/ (forces full rebuild)\n" "make clean-tests"
	@printf "    %-$(HELP_W)s  Compare rvsim with gem5 on the benchmark set (GEM5_BIN=…)\n" "make compare-gem5"
	@printf "\n  $(CYAN)Run$(RESET)\n"
	@printf "    %-$(HELP_W)s  Build and run quicksort benchmark\n" "make run-example"
	@printf "    %-$(HELP_W)s  Boot SMP Linux: 8 M-class O3 cores, TAGE-SC-L, snoop filter, mesh, 4-ch DDR5 (HARTS=N)\n" "make run-linux"
	@printf "\n  $(CYAN)Profiling$(RESET)\n"
	@printf "    %-$(HELP_W)s  Build with profiling symbols\n" "make profile-build"
	@printf "    %-$(HELP_W)s  Generate flamegraph (ARGS=…)\n" "make flamegraph"
	@printf "\n  $(CYAN)Housekeeping$(RESET)\n"
	@printf "    %-$(HELP_W)s  Remove all build artifacts\n" "make clean"
	@printf "    %-$(HELP_W)s  Remove Rust artifacts only\n" "make clean-rust"
	@printf "    %-$(HELP_W)s  Remove Python build artifacts\n" "make clean-python"
	@printf "    %-$(HELP_W)s  Remove software artifacts only\n" "make clean-software"
	@printf "\n"

build: python

software:
	@printf "$(GREEN)Building libc and example programs…$(RESET)\n"
	$(MAKE) -C software

examples: software

linux:
	@printf "$(GREEN)Building Linux kernel and rootfs…$(RESET)\n"
ifeq ($(RVSIM_FHS_ACTIVE),)
	nix run .\#fhs -- -c '$(MAKE) -C software linux'
else
	$(MAKE) -C software linux
endif

# Install Python bindings in editable/dev mode via maturin
python:
	@printf "$(GREEN)Installing Python bindings (editable)…$(RESET)\n"
	@if [ ! -d .venv ]; then \
		printf "$(GREEN)Creating .venv…$(RESET)\n"; \
		python3 -m venv .venv; \
	fi
	@.venv/bin/pip install --quiet -r requirements-dev.txt
	.venv/bin/maturin develop --release

# Build a distributable wheel into target/wheels
python-wheel:
	@printf "$(GREEN)Building Python wheel into $(BUILD_DIR)/wheels…$(RESET)\n"
	$(MATURIN) build --release --out $(BUILD_DIR)/wheels

check:
	@printf "$(GREEN)Running cargo check…$(RESET)\n"
	$(CARGO) check --workspace --all-targets

test:
	@printf "$(GREEN)Running Rust tests…$(RESET)\n"
	$(CARGO) test --workspace

test-python: python
	@printf "$(GREEN)Running Python API tests…$(RESET)\n"
	.venv/bin/python -m unittest discover -s tests/python
	@printf "$(GREEN)Checking rvsim/_core.pyi against the extension…$(RESET)\n"
	.venv/bin/python -m mypy.stubtest rvsim._core

# Runs gem5 only when it is available; otherwise compares with the stored
# tools/gem5_compare/results/gem5.json.
GEM5_BIN ?= $(shell command -v gem5.opt)
compare-gem5: python
	bash tools/gem5_compare/programs/build.sh
	@printf "$(GREEN)Running rvsim on the comparison set…$(RESET)\n"
	$(PYTHON) tools/gem5_compare/run_rvsim.py
	@if [ -n "$(GEM5_BIN)" ]; then \
		printf "$(GREEN)Running gem5…$(RESET)\n"; \
		GEM5_BIN="$(GEM5_BIN)" $(PYTHON) tools/gem5_compare/run_gem5.py; \
	else \
		printf "$(BOLD)gem5.opt not found; comparing with the stored gem5 results.$(RESET)\n"; \
	fi
	$(PYTHON) tools/gem5_compare/compare.py

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
	$(PYTHON) -m ruff format rvsim tests examples tools docs

fmt-check:
	@printf "$(GREEN)Checking Rust formatting…$(RESET)\n"
	$(CARGO) fmt --all -- --check
	@printf "$(GREEN)Checking Python formatting…$(RESET)\n"
	$(PYTHON) -m ruff format --check rvsim tests examples tools docs

lint: fmt-check clippy

SPIKE_LOCAL    := $(TEST_BUILDS)/spike-install/bin/spike
VECTOR_PATTERN ?= .*
VECTOR_VLEN    ?= 128

include tests/conformance/sources.mk

# $(call clone-at,url,rev,dir): a shallow checkout of one commit.
clone-at = git init -q $(3) && git -C $(3) fetch -q --depth 1 $(1) $(2) && \
	git -C $(3) checkout -q FETCH_HEAD


$(SPIKE_LOCAL):
	@printf "$(GREEN)Building local spike from source (one-time, ~2 min)…$(RESET)\n"
	@mkdir -p $(TEST_BUILDS)
	@if [ ! -d $(TEST_BUILDS)/spike-src ]; then \
		$(call clone-at,https://github.com/riscv-software-src/riscv-isa-sim.git,$(SPIKE_REV),$(TEST_BUILDS)/spike-src); \
	fi
	@mkdir -p $(TEST_BUILDS)/spike-build
	@cd $(TEST_BUILDS)/spike-build && \
		../spike-src/configure --prefix=$$(pwd)/../spike-install >/dev/null && \
		$(MAKE) -j$$(nproc) install >/dev/null

# riscv-tests source + build (was software/riscv-tests)
$(TEST_BUILDS)/riscv-tests:
	@printf "$(GREEN)Cloning riscv-tests…$(RESET)\n"
	@mkdir -p $(TEST_BUILDS)
	$(call clone-at,https://github.com/riscv-software-src/riscv-tests.git,$(RISCV_TESTS_REV),$(TEST_BUILDS)/riscv-tests)
	cd $(TEST_BUILDS)/riscv-tests && git submodule update --init --recursive

RISCV_TESTS_STAMP := $(TEST_BUILDS)/riscv-tests/.built-p

riscv-tests-build: $(RISCV_TESTS_STAMP)

# Build only the -p- (physical-mode) variants. The -v- variants need libc
# headers (string.h, stdint.h) that the bare-metal toolchain doesn't ship; we
# don't run them anyway. Output is silenced to a log; if anything matters the
# stamp file won't exist after the build.
$(RISCV_TESTS_STAMP): $(TEST_BUILDS)/riscv-tests
	@printf "$(GREEN)Building riscv-tests -p- ELFs (this is noisy, logging to .build.log)…$(RESET)\n"
	@-$(MAKE) -k RISCV_PREFIX=riscv64-elf- \
	    -C $(TEST_BUILDS)/riscv-tests/isa XLEN=64 \
	    > $(TEST_BUILDS)/riscv-tests/.build.log 2>&1
	@n=$$(find $(TEST_BUILDS)/riscv-tests/isa -maxdepth 1 -type f \
	      \( -name 'rv64*-p-*' -o -name 'rv32*-p-*' \) ! -name '*.dump' | wc -l); \
	  printf "$(GREEN)Built $$n -p- test ELFs$(RESET)\n"; \
	  if [ "$$n" -gt 0 ]; then touch $(RISCV_TESTS_STAMP); fi

riscv-tests: riscv-tests-build python
	@printf "$(GREEN)Running riscv-tests across all PIPELINES…$(RESET)\n"
	.venv/bin/python tests/conformance/riscv_tests.py

# Vector tests (chipsalliance generator + spike cosim)
vector-test-build: $(SPIKE_LOCAL)
	@printf "$(GREEN)Building RVV test ELFs (VLEN=$(VECTOR_VLEN), pattern='$(VECTOR_PATTERN)')…$(RESET)\n"
	@VLEN=$(VECTOR_VLEN) PATTERN='$(VECTOR_PATTERN)' bash tests/conformance/vector/build_tests.sh

vector-test: vector-test-build python
	@printf "$(GREEN)Running RVV cosim suite (rvsim vs spike)…$(RESET)\n"
	.venv/bin/python tests/conformance/vector/run_vector_tests.py --vlen $(VECTOR_VLEN)

# The smoke sample builds into its own directory, so it leaves a full build
# in place.
vector-smoke-build: $(SPIKE_LOCAL)
	@printf "$(GREEN)Building the RVV smoke sample (VLEN=$(VECTOR_VLEN))…$(RESET)\n"
	@VLEN=$(VECTOR_VLEN) PATTERN='$(VECTOR_SMOKE_PATTERN)' VECTOR_BUILD=$(abspath $(VECTOR_SMOKE_BUILD)) \
		bash tests/conformance/vector/build_tests.sh

vector-test-smoke: vector-smoke-build python
	@printf "$(GREEN)Running the RVV smoke sample (rvsim vs spike)…$(RESET)\n"
	.venv/bin/python tests/conformance/vector/run_vector_tests.py --vlen $(VECTOR_VLEN) \
		--build-dir $(VECTOR_SMOKE_BUILD)

# Multi-config runners
# Each runs every test in its suite across every Config in
# tests/conformance/configs/pipelines.py.

vector-test-multi: vector-test-build python
	@printf "$(GREEN)Running RVV tests across all PIPELINES (this is slow)…$(RESET)\n"
	.venv/bin/python tests/conformance/vector_tests.py --vlen $(VECTOR_VLEN)

# The big one
# Builds everything, runs every suite × every PIPELINES config, prints unified
# summary, exits non-zero on any failure. Several CPU-hours.
test-all: riscv-tests-build vector-test-build software python
	@printf "$(GREEN)Running ALL tests across ALL pipeline configs…$(RESET)\n"
	.venv/bin/python tests/run_all.py

# The conformance suites' smoke subsets: every riscv-test on the smoke
# configs, the RVV sample against spike, and the multi-hart programs.
conformance-smoke: riscv-tests-build vector-smoke-build software python
	.venv/bin/python tests/run_all.py --smoke

# What CI requires before a merge: Rust and Python tests, then the
# conformance smoke.
test-all-smoke: test test-python conformance-smoke

# Wipe everything under tests/builds/ — forces full rebuild on next run.
clean-tests:
	@printf "$(GREEN)Removing $(TEST_BUILDS) (all test build artifacts)…$(RESET)\n"
	rm -rf $(TEST_BUILDS)

prerelease:
	@tools/prerelease $(PRERELEASE_ARGS)

run-example: software
	@printf "$(GREEN)Running quicksort benchmark…$(RESET)\n"
	.venv/bin/rvsim -f software/bin/benchmarks/qsort.elf

HARTS ?= 8
run-linux:
	@test -x .venv/bin/rvsim || $(MAKE) python
	@test -f software/linux/output/Image -a -f software/linux/output/fw_jump.bin || $(MAKE) linux
	@printf "$(GREEN)Booting Linux on $(HARTS) hart(s) with DDR5-5600 over a mesh…$(RESET)\n"
	.venv/bin/rvsim tools/boot_linux.py --no-build --harts $(HARTS)

profile-build:
	@printf "$(GREEN)Building with profiling symbols…$(RESET)\n"
	.venv/bin/maturin develop --profile profiling

flamegraph:
	@printf "$(GREEN)Recording flamegraph…$(RESET)\n"
	$$HOME/.cargo/bin/flamegraph -o flamegraph.svg -F 99 -- .venv/bin/rvsim $(ARGS)

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
