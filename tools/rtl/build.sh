#!/usr/bin/env bash
# Builds the Chipyard Verilator simulators tools/diag/rtl_compare.py runs
# against, in tests/builds/chipyard: Rocket (RocketConfig) and BOOM
# (RvsimMediumBoomV4Config, tools/rtl/RvsimConfigs.scala). Run inside the
# rtl dev shell (`nix develop .#rtl`), which provides the JDK, firtool,
# Verilator and `RISCV` for the fesvr library; `make rtl-build` does.
#
# Usage: tools/rtl/build.sh [CONFIG ...]    (default: both configs)
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
chipyard="$root/tests/builds/chipyard"
rev="$(sed -n 's/^CHIPYARD_REV *:= *//p' "$root/tests/conformance/sources.mk")"
configs=("$@")
if [ ${#configs[@]} -eq 0 ]; then
    configs=(RocketConfig RvsimMediumBoomV4Config)
fi

if [ -z "${RISCV:-}" ] || [ ! -f "$RISCV/lib/libfesvr.a" ]; then
    echo "RISCV must point at a spike install with libfesvr (nix develop .#rtl sets it)" >&2
    exit 1
fi

if [ ! -d "$chipyard/.git" ]; then
    echo "Cloning Chipyard at $rev into $chipyard"
    git init -q "$chipyard"
    git -C "$chipyard" fetch -q --depth 1 https://github.com/ucb-bar/chipyard.git "$rev"
    git -C "$chipyard" checkout -q FETCH_HEAD
fi
if [ "$(git -C "$chipyard" rev-parse HEAD)" != "$rev" ]; then
    echo "$chipyard is at $(git -C "$chipyard" rev-parse HEAD), not the pinned $rev; move it aside" >&2
    exit 1
fi

if [ ! -f "$chipyard/generators/boom/build.sbt" ]; then
    echo "Initialising Chipyard's submodules"
    (cd "$chipyard" && ./scripts/init-submodules-no-riscv-tools-nolog.sh)
fi

boom="$chipyard/generators/boom"
for patch in "$root"/tools/rtl/patches/boom-*.patch; do
    if git -C "$boom" apply --check --reverse "$patch" 2>/dev/null; then
        continue # already applied
    fi
    echo "Applying $(basename "$patch")"
    git -C "$boom" apply "$patch"
done

cp "$root/tools/rtl/RvsimConfigs.scala" "$chipyard/generators/chipyard/src/main/scala/config/RvsimConfigs.scala"

for config in "${configs[@]}"; do
    echo "Building the $config Verilator simulator"
    make -C "$chipyard/sims/verilator" CONFIG="$config" VERILATOR_THREADS="${VERILATOR_THREADS:-4}" -j"$(nproc)"
done
ls -1 "$chipyard"/sims/verilator/simulator-chipyard.harness-*
