#!/usr/bin/env bash
# Buildroot post-build script: compiles the guest tools with the toolchain
# Buildroot just built and installs them into the target root filesystem,
# with the sysctl settings the measurements need.
#
# $1 is the target directory; Buildroot exports HOST_DIR and BR2_DL_DIR.
set -euo pipefail

target_dir=$1
guest_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
include_dir=$(cd "$guest_dir/../include" && pwd)
cc=$(ls "$HOST_DIR"/bin/riscv64-buildroot-linux-*-gcc | head -n1)

"$cc" -static -O2 -Wall -Wextra -I"$include_dir" -o "$target_dir/usr/bin/rvsim" "$guest_dir/rvsim.c"

# Linux 6.6 denies rdcycle and rdinstret to user space unless perf lets
# them through; 2 opens the counters to every process, as before 6.6.
mkdir -p "$target_dir/etc/sysctl.d"
printf 'kernel.perf_user_access = 2\n' > "$target_dir/etc/sysctl.d/10-rvsim.conf"

# STREAM's arrays (3 x 32 MB) exceed the showcase's 36 MB L3, so it measures
# memory bandwidth; two passes keep a detailed run to minutes.
stream_src=$BR2_DL_DIR/stream/stream.c
if [ ! -f "$stream_src" ]; then
    mkdir -p "$(dirname "$stream_src")"
    curl -sSfL -o "$stream_src" https://www.cs.virginia.edu/stream/FTP/Code/stream.c
fi
"$cc" -static -O2 -DSTREAM_ARRAY_SIZE=4000000 -DNTIMES=2 -o "$target_dir/usr/bin/stream" "$stream_src"
