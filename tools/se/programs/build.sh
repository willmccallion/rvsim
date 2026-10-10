#!/usr/bin/env bash
# Builds the SE mode check programs (tools/se/programs/*.c) as static
# riscv64 Linux executables with the dev shell's riscv64-linux-gnu-gcc,
# into tests/builds/se-programs/<name>.elf.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$root/tests/builds/se-programs"
cc="${CC_RISCV_LINUX:-riscv64-linux-gnu-gcc}"

mkdir -p "$out"
for src in "$here"/*.c; do
    name="$(basename "$src" .c)"
    "$cc" -std=gnu17 -O2 -static -march=rv64gc -mabi=lp64d -Wall "$src" -o "$out/$name.elf"
done
echo "Built $(ls "$out"/*.elf | wc -l) SE check programs in $out"
