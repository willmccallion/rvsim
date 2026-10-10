#!/usr/bin/env bash
# Builds the RTL comparison's workload set into tests/builds/rtl-programs/:
# the scalar gem5 comparison kernels (tools/gem5_compare/programs) and the
# one-mechanism microbenchmarks here, each linked with tools/rtl/programs/
# crt0.s so it exits through HTIF. The ISA is what Rocket and BOOM both
# implement: rv64gc with Zba, Zbb and Zbs.
set -euo pipefail
shopt -s nullglob
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="$root/tests/builds/rtl-programs"
mkdir -p "$out"
cflags=(-march=rv64gc_zba_zbb_zbs -mabi=lp64d -mcmodel=medany -ffreestanding -nostdlib -O2 -g)
riscv64-elf-gcc "${cflags[@]}" -c "$here/crt0.s" -o "$out/crt0.o"
build() {
    local src="$1" name
    name="$(basename "$src" .c)"
    riscv64-elf-gcc "${cflags[@]}" -I"$root/tools/gem5_compare/programs" -c "$src" -o "$out/$name.o"
    riscv64-elf-ld --no-warn-rwx-segments -T "$here/rtl.ld" "$out/crt0.o" "$out/$name.o" -o "$out/$name.elf"
}
for src in "$root"/tools/gem5_compare/programs/*.c; do
    case "$(basename "$src")" in vec_*) continue ;; esac
    build "$src"
done
for src in "$here"/*.c; do
    build "$src"
done
ls "$out"/*.elf | wc -l
