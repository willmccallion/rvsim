#!/usr/bin/env bash
# Build a comprehensive set of RVV 1.0 test ELFs from the chipsalliance
# riscv-vector-tests generator. We use stage1 .S files (no pspike needed):
# the magic instruction `.word 0x2a000b` is stripped, and pass/fail is
# determined later by diffing the post-execution memory signature against
# spike running the same ELF (cosim).
#
# Env knobs:
#   VLEN         (default 128) — power of 2 in [64..4096]
#   XLEN         (default 64)  — 32 or 64
#   PATTERN      (default '.*') — regex filtering instruction names
#   MARCH        (default chosen from spike capabilities)
#   JOBS         (default $(nproc))

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
TESTING=$(cd "$HERE/.." && pwd)
BUILDS=$TESTING/builds
GEN_DIR=$BUILDS/riscv-vector-tests
SPIKE_DIR=$BUILDS/spike-install
BUILD=$BUILDS/vector

VLEN=${VLEN:-128}
XLEN=${XLEN:-64}
PATTERN=${PATTERN:-.*}
JOBS=${JOBS:-$(nproc)}

# Spike from source supports the modern Z-extensions; system spike (Arch 1.1.0)
# does not. Default MARCH targets the local spike build if available.
if [ -x "$SPIKE_DIR/bin/spike" ]; then
  DEFAULT_MARCH="rv${XLEN}gcv_zfh_zvfh_zvbb_zvbc_zvkg_zvkned_zvknha_zvksed_zvksh"
else
  DEFAULT_MARCH="rv${XLEN}gcv_zfh"
fi
MARCH=${MARCH:-$DEFAULT_MARCH}

if [ "$XLEN" = "64" ]; then
  MABI=lp64d
else
  MABI=ilp32f
fi

ENV_DIR=$GEN_DIR/env/riscv-test-env/p
MACROS_DIR=$GEN_DIR/macros/general
LINK_LD=$ENV_DIR/link.ld

GREEN='\033[32m'; CYAN='\033[36m'; RESET='\033[0m'
log() { printf "${GREEN}[vector]${RESET} %s\n" "$*"; }

# ── 1. Clone generator if needed ─────────────────────────────────────────────
if [ ! -d "$GEN_DIR" ]; then
  log "Cloning chipsalliance/riscv-vector-tests"
  mkdir -p "$BUILDS"
  git clone --depth 1 https://github.com/chipsalliance/riscv-vector-tests.git "$GEN_DIR"
fi

# Make sure the env submodule is checked out (riscv-test-env).
if [ ! -f "$LINK_LD" ]; then
  log "Initializing riscv-test-env submodule"
  git -C "$GEN_DIR" submodule update --init --recursive
fi

# ── 2. Build the Go generator ────────────────────────────────────────────────
if [ ! -x "$GEN_DIR/build/generator" ]; then
  log "Building Go generator (one-time)"
  (cd "$GEN_DIR" && go build -o build/generator)
fi

# ── 3. Generate stage1 .S files ──────────────────────────────────────────────
STAGE1=$BUILD/stage1
mkdir -p "$STAGE1"
log "Generating stage1 tests (VLEN=$VLEN XLEN=$XLEN pattern='$PATTERN')"
log "  MARCH=$MARCH"
(cd "$GEN_DIR" && \
  build/generator \
    -VLEN "$VLEN" \
    -XLEN "$XLEN" \
    -split=10000 \
    -integer=0 \
    -pattern="$PATTERN" \
    -testfloat3level=1 \
    -repeat=1 \
    -stage1output "$STAGE1/" \
    -configs configs \
    -march "$MARCH")

NUM_S=$(find "$STAGE1" -name '*.S' -type f | wc -l)
log "Generated $NUM_S stage1 .S files"

if [ "$NUM_S" -eq 0 ]; then
  echo "ERROR: generator produced 0 tests; check pattern/march" >&2
  exit 1
fi

# ── 4. Strip magic insns + compile in parallel ───────────────────────────────
ELF_DIR=$BUILD/vlen${VLEN}
mkdir -p "$ELF_DIR"

compile_one() {
  local s=$1
  local name
  name=$(basename "$s" .S)
  local clean=$ELF_DIR/$name.S
  local elf=$ELF_DIR/$name.elf
  # Strip ALL chipsalliance magic instructions: any `.word 0x...b` (custom0
  # opcode 0x0B). pspike would replace these with TEST_CASE expected-value
  # checks; in cosim mode we just remove them and diff the resultdata against
  # spike's run.
  sed -E '/^\.word 0x[0-9a-fA-F]+b$/d' "$s" > "$clean"
  riscv64-elf-gcc -march="$MARCH" -mabi="$MABI" \
    -static -mcmodel=medany -fvisibility=hidden -nostdlib -nostartfiles \
    -DENTROPY=0xdeadbeef -DLFSR_BITS=9 -fno-tree-loop-distribute-patterns \
    -I"$ENV_DIR" -I"$MACROS_DIR" \
    -T"$LINK_LD" \
    "$clean" -o "$elf" 2>&1 | sed "s|^|[$name] |" >&2
  rm -f "$clean"
}
export -f compile_one
export ELF_DIR ENV_DIR MACROS_DIR LINK_LD MARCH MABI

log "Compiling $NUM_S tests with $JOBS parallel jobs"
find "$STAGE1" -name '*.S' -type f -print0 \
  | xargs -0 -n1 -P"$JOBS" bash -c 'compile_one "$0"'

NUM_ELF=$(find "$ELF_DIR" -name '*.elf' -type f | wc -l)
log "Built $NUM_ELF / $NUM_S ELFs into $ELF_DIR"
if [ "$NUM_ELF" -lt "$NUM_S" ]; then
  echo "WARNING: $((NUM_S - NUM_ELF)) tests failed to compile" >&2
fi
