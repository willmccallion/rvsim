#!/usr/bin/env python3
"""
Download Buildroot, build Linux (kernel + rootfs + OpenSBI), then boot in the simulator.

Everything runs under software/linux: download, build, and artifacts (output/).
Run from repo root:

  sim script scripts/setup/boot_linux.py            # build if needed, then boot
  sim script scripts/setup/boot_linux.py --no-boot  # only download & build
  sim script scripts/setup/boot_linux.py --no-build # boot only (fail if no Image)
"""

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request

from rvsim import (
    Backend,
    BranchPredictor,
    Cache,
    Config,
    Fu,
    MemoryController,
    Prefetcher,
    ReplacementPolicy,
    Simulator,
)

BUILDROOT_VER = "2024.08"
BUILDROOT_URL = f"https://buildroot.org/downloads/buildroot-{BUILDROOT_VER}.tar.gz"
DEFCONFIG = """BR2_riscv=y
BR2_RISCV_64=y
BR2_RISCV_ISA_RVC=y
BR2_RISCV_ABI_LP64D=y
BR2_LINUX_KERNEL=y
BR2_LINUX_KERNEL_CUSTOM_VERSION=y
BR2_LINUX_KERNEL_CUSTOM_VERSION_VALUE="6.6.44"
BR2_LINUX_KERNEL_USE_ARCH_DEFAULT_CONFIG=y
BR2_TARGET_OPENSBI=y
BR2_TARGET_OPENSBI_PLAT="generic"
BR2_TARGET_OPENSBI_ADDITIONAL_VARIABLES="PLATFORM_RISCV_ISA=rv64imafdc_zifencei"
BR2_TARGET_ROOTFS_EXT2=y
BR2_TARGET_ROOTFS_EXT2_SIZE="60M"
BR2_PACKAGE_HOST_LINUX_HEADERS_CUSTOM_6_6=y
"""



def repo_root():
    return os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def download_buildroot(linux_dir: str, buildroot_dir: str) -> None:
    tarball = os.path.join(linux_dir, f"buildroot-{BUILDROOT_VER}.tar.gz")
    if os.path.isdir(buildroot_dir):
        print("[Linux] Using existing Buildroot tree:", buildroot_dir)
        return
    if not os.path.isfile(tarball):
        print("[Linux] Downloading Buildroot", BUILDROOT_VER, "...")
        urllib.request.urlretrieve(BUILDROOT_URL, tarball)
    print("[Linux] Extracting Buildroot...")
    with tarfile.open(tarball, "r:gz") as tf:
        try:
            tf.extractall(linux_dir, filter="fully_trusted")
        except TypeError:
            tf.extractall(linux_dir)


def write_defconfig(buildroot_dir: str) -> None:
    configs_dir = os.path.join(buildroot_dir, "configs")
    os.makedirs(configs_dir, exist_ok=True)
    path = os.path.join(configs_dir, "riscv_emu_defconfig")
    with open(path, "w") as f:
        f.write(DEFCONFIG)
    print("[Linux] Wrote", path)



def build(linux_dir: str) -> int:
    """Download, configure, and build Buildroot + compile DTB. Returns 0 on success."""
    buildroot_dir = os.path.join(linux_dir, f"buildroot-{BUILDROOT_VER}")
    download_buildroot(linux_dir, buildroot_dir)
    write_defconfig(buildroot_dir)

    env = os.environ.copy()
    env["HOST_CFLAGS"] = (
        "-O2 -std=gnu11 -Wno-implicit-function-declaration -Wno-int-conversion -Wno-incompatible-pointer-types -Wno-return-type -Wno-error"
    )

    print("[Linux] Configuring Buildroot...")
    r = subprocess.run(
        ["make", "riscv_emu_defconfig"],
        cwd=buildroot_dir,
        env=env,
    )
    if r.returncode != 0:
        return r.returncode

    print("[Linux] Building (this may take a while)...")
    nproc = os.cpu_count() or 4
    r = subprocess.run(
        ["make", f"-j{nproc}"],
        cwd=buildroot_dir,
        env=env,
    )
    if r.returncode != 0:
        return r.returncode

    out_dir = os.path.join(linux_dir, "output")
    os.makedirs(out_dir, exist_ok=True)
    br_images = os.path.join(buildroot_dir, "output", "images")
    shutil.copy(os.path.join(br_images, "Image"), os.path.join(out_dir, "Image"))
    shutil.copy(
        os.path.join(br_images, "rootfs.ext2"), os.path.join(out_dir, "disk.img")
    )
    shutil.copy(
        os.path.join(br_images, "fw_jump.bin"), os.path.join(out_dir, "fw_jump.bin")
    )
    shutil.copy(
        os.path.join(br_images, "fw_dynamic.bin"),
        os.path.join(out_dir, "fw_dynamic.bin"),
    )
    print("[Linux] Copied Image, disk.img, fw_jump.bin, fw_dynamic.bin to", out_dir)

    print("[Linux] Build complete.")
    return 0


def config() -> Config:
    """Maximum-performance config for Linux boot.

    8-wide O3 superscalar, 8-bank TAGE, Inclusive 4-level cache hierarchy with
    aggressive prefetching at every level, large L2 TLB, and a DRAM controller.
    """
    return Config(
        # ── Frontend ──────────────────────────────────────────────────────────
        width=8,
        branch_predictor=BranchPredictor.TAGE(
            num_banks=8,
            table_size=8192,
            loop_table_size=1024,
            reset_interval=500_000,
            history_lengths=[5, 11, 22, 44, 89, 178, 356, 712],
            tag_widths=[9, 9, 10, 10, 11, 11, 12, 12],
        ),
        btb_size=16384,
        btb_ways=8,
        ras_size=128,
        # ── Out-of-order backend ──────────────────────────────────────────────
        backend=Backend.OutOfOrder(
            rob_size=256,
            store_buffer_size=64,
            issue_queue_size=96,
            load_queue_size=64,
            load_ports=4,
            store_ports=2,
            prf_gpr_size=512,
            prf_fpr_size=256,
            fu_config=Fu(
                [
                    Fu.IntAlu(count=6, latency=1),
                    Fu.IntMul(count=2, latency=3),
                    Fu.IntDiv(count=2, latency=20),
                    Fu.FpAdd(count=4, latency=4),
                    Fu.FpMul(count=4, latency=5),
                    Fu.FpFma(count=4, latency=5),
                    Fu.FpDivSqrt(count=2, latency=21),
                    Fu.Branch(count=4, latency=1),
                    Fu.Mem(count=4, latency=1),
                ]
            ),
        ),
        # ── Cache hierarchy ───────────────────────────────────────────────────
        l1i=Cache(
            size="64KB",
            line="64B",
            ways=8,
            policy=ReplacementPolicy.PLRU(),
            latency=1,
            prefetcher=Prefetcher.NextLine(degree=4),
            mshr_count=8,
        ),
        l1d=Cache(
            size="64KB",
            line="64B",
            ways=8,
            policy=ReplacementPolicy.PLRU(),
            latency=1,
            prefetcher=Prefetcher.Stride(degree=4, table_size=256),
            mshr_count=16,
        ),
        l2=Cache(
            size="2MB",
            line="64B",
            ways=16,
            policy=ReplacementPolicy.PLRU(),
            latency=8,
            prefetcher=Prefetcher.Stream(degree=8),
            mshr_count=32,
        ),
        l3=Cache(
            size="16MB",
            line="64B",
            ways=16,
            policy=ReplacementPolicy.PLRU(),
            latency=24,
            prefetcher=Prefetcher.Tagged(degree=4),
            mshr_count=64,
        ),
        inclusion_policy=Cache.Inclusive(),
        wcb_entries=16,
        # ── Memory ────────────────────────────────────────────────────────────
        ram_size="256MB",
        tlb_size=256,
        l2_tlb_size=2048,
        l2_tlb_ways=8,
        l2_tlb_latency=3,
        memory_controller=MemoryController.Simple(),
        # ── System addresses (must match device tree) ─────────────────────────
        ram_base=0x80000000,
        uart_base=0x10000000,
        disk_base=0x10001000,
        clint_base=0x02000000,
        syscon_base=0x00100000,
        kernel_offset=0x200000,
        bus_width=8,
        bus_latency=1,
        clint_divider=1,
    )


def main():
    root = repo_root()
    linux_dir = os.path.join(root, "software", "linux")
    out_dir = os.path.join(linux_dir, "output")
    image_path = os.path.join(out_dir, "Image")
    disk_path = os.path.join(out_dir, "disk.img")

    ap = argparse.ArgumentParser(
        description="Download Buildroot, build Linux, optionally boot in sim"
    )
    ap.add_argument(
        "--no-build", action="store_true", help="Skip build; fail if Image missing"
    )
    ap.add_argument(
        "--no-boot", action="store_true", help="Only build; do not run simulator"
    )
    args = ap.parse_args()

    if not args.no_build:
        if not os.path.exists(image_path) or not os.path.exists(
            os.path.join(out_dir, "fw_jump.bin")
        ):
            os.makedirs(linux_dir, exist_ok=True)
            if build(linux_dir) != 0:
                return 1
        else:
            print("[boot_linux] Using existing Linux artifacts in", out_dir)
    else:
        if not os.path.exists(image_path):
            print(
                "Error: Image not found at",
                image_path,
                "(run without --no-build to build)",
            )
            return 1

    if args.no_boot:
        return 0

    if not os.path.exists(disk_path):
        print("Error: disk image not found:", disk_path)
        return 1

    os.chdir(root)

    print("[boot_linux] Booting with Simulator (Optimized Config)...")

    sim = Simulator().config(config()).kernel(image_path).disk(disk_path)

    try:
        return sim.run(
            limit=100_000_000_000,
        )  # Add progress = ... to this if it seems to hang.
    except Exception as e:
        print(f"Simulation failed: {e}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
