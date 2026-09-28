#!/usr/bin/env bash
# Builds the comparison programs into testing/builds/compare-programs/.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$root/testing/builds/compare-programs"
mkdir -p "$out"
cflags=(-march=rv64gcv_zba_zbb_zbc_zbs_zbkb_zbkx_zfh -mabi=lp64d -mcmodel=medany -ffreestanding -nostdlib -O2 -g)
riscv64-elf-gcc "${cflags[@]}" -c "$root/software/libc/crt0.s" -o "$out/crt0.o"
for src in "$here"/*.c; do
    name="$(basename "$src" .c)"
    riscv64-elf-gcc "${cflags[@]}" -c "$src" -o "$out/$name.o"
    riscv64-elf-ld --no-warn-rwx-segments -T "$root/software/libc/user.ld" "$out/crt0.o" "$out/$name.o" -o "$out/$name.elf"
done
ls "$out"/*.elf | wc -l
