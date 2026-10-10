#!/usr/bin/env bash
# Builds Embench-IoT as static riscv64 Linux programs for SE mode.
#
# Clones embench-iot at EMBENCH_REV (tests/conformance/sources.mk) into
# tests/builds/embench and links each benchmark with the harness and
# the rvsim board (tools/embench/board) using the dev shell's
# riscv64-linux-gnu-gcc, into tests/builds/embench/bin/<name>.elf. A
# benchmark exits 0 when its result verified. Embench is GPL-3.0: it is
# built here on demand and never distributed with rvsim.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
src="$root/tests/builds/embench"
out="$src/bin"
rev="${EMBENCH_REV:?set EMBENCH_REV (see tests/conformance/sources.mk)}"
cc="${CC_RISCV_LINUX:-riscv64-linux-gnu-gcc}"

if [ ! -f "$src/sconstruct.py" ]; then
    echo "Cloning embench-iot at $rev"
    mkdir -p "$src"
    git init -q "$src"
    git -C "$src" fetch -q --depth 1 https://github.com/embench/embench-iot "$rev"
    git -C "$src" checkout -q FETCH_HEAD
fi

mkdir -p "$out"
cflags=(-std=gnu17 -O2 -static -march=rv64gc -mabi=lp64d
        -ffunction-sections -fdata-sections -Wl,--gc-sections
        -DWARMUP_HEAT=1 -DGLOBAL_SCALE_FACTOR=1 -DHAVE_BOARDSUPPORT_H
        -I"$src/support" -I"$here/board")
built=0
for dir in "$src"/src/*/; do
    name="$(basename "$dir")"
    sources=("$dir"*.c)
    [ -e "${sources[0]}" ] || continue
    "$cc" "${cflags[@]}" "${sources[@]}" \
        "$src/support/main.c" "$src/support/beebsc.c" "$here/board/boardsupport.c" \
        -lm -o "$out/$name.elf"
    built=$((built + 1))
done
echo "Built $built Embench programs in $out"
