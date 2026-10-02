#!/usr/bin/env python3
"""
Download Buildroot, build Linux (kernel + rootfs + OpenSBI), then boot in the simulator.

Everything runs under software/linux: download, build, and artifacts (output/).
Run from repo root:

  sim script tools/boot_linux.py            # build if needed, then boot
  sim script tools/boot_linux.py --no-boot  # only download & build
  sim script tools/boot_linux.py --no-build # boot only (fail if no Image)
  sim script tools/boot_linux.py --rebuild --no-boot  # rebuild after package changes

The root filesystem carries the benchmark suite (CoreMark, CoreMark-PRO,
Dhrystone, Whetstone, lmbench, mbw, ramspeed, stress-ng, STREAM, perf) and
the ``rvsim`` guest tool that marks regions for the simulator's stats.

The default boot is the showcase system: eight out-of-order M-class cores
from the ``fast`` preset (64KB TAGE-SC-L), a MESI snoop-filter home agent
over a 2-D mesh, and four channels of DDR5-5600. ``--harts``, ``--memory``, ``--speed-bin`` and
``--interconnect`` trim it down.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request

from rvsim import Simulator, presets
from rvsim.presets import INTERCONNECTS

BUILDROOT_VER = "2024.08"
BUILDROOT_URL = f"https://buildroot.org/downloads/buildroot-{BUILDROOT_VER}.tar.gz"
DEFCONFIG = """BR2_riscv=y
BR2_RISCV_64=y
BR2_RISCV_ISA_RVC=y
BR2_RISCV_ISA_RVV=y
BR2_RISCV_ABI_LP64D=y
BR2_LINUX_KERNEL=y
BR2_LINUX_KERNEL_CUSTOM_VERSION=y
BR2_LINUX_KERNEL_CUSTOM_VERSION_VALUE="6.6.44"
BR2_LINUX_KERNEL_USE_ARCH_DEFAULT_CONFIG=y
BR2_TARGET_OPENSBI=y
BR2_TARGET_OPENSBI_PLAT="generic"
BR2_TARGET_OPENSBI_ADDITIONAL_VARIABLES="PLATFORM_RISCV_ISA=rv64imafdcv_zifencei"
BR2_TARGET_ROOTFS_EXT2=y
BR2_TARGET_ROOTFS_EXT2_SIZE="256M"
BR2_PACKAGE_HOST_LINUX_HEADERS_CUSTOM_6_6=y
BR2_ROOTFS_POST_BUILD_SCRIPT="{guest}/post_build.sh"
BR2_GLOBAL_PATCH_DIR="{guest}/buildroot-patches"
BR2_PACKAGE_COREMARK=y
BR2_PACKAGE_COREMARK_PRO=y
BR2_PACKAGE_DHRYSTONE=y
BR2_PACKAGE_WHETSTONE=y
BR2_PACKAGE_LMBENCH=y
BR2_PACKAGE_MBW=y
BR2_PACKAGE_RAMSPEED=y
BR2_PACKAGE_STRESS_NG=y
BR2_PACKAGE_LINUX_TOOLS_PERF=y
"""


HOST_TOOLS = (
    "bash",
    "bc",
    "bison",
    "bzip2",
    "cpio",
    "file",
    "flex",
    "g++",
    "gcc",
    "gzip",
    "ld",
    "make",
    "patch",
    "perl",
    "rsync",
    "sed",
    "tar",
    "unzip",
    "wget",
    "which",
)


def repo_root():
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def missing_host_tools() -> list[str]:
    """The Buildroot host prerequisites not on PATH."""
    return [tool for tool in HOST_TOOLS if shutil.which(tool) is None]


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
    guest = os.path.join(repo_root(), "software", "guest")
    with open(path, "w") as f:
        f.write(DEFCONFIG.format(guest=guest))
    print("[Linux] Wrote", path)


def build(linux_dir: str) -> int:
    """Download, configure, and build Buildroot + compile DTB. Returns 0 on success."""
    missing = missing_host_tools()
    if missing:
        print("[Linux] Missing host tools Buildroot needs:", ", ".join(missing))
        print(
            "[Linux] Install them, or run `make linux` to build inside the Nix FHS shell."
        )
        return 1
    buildroot_dir = os.path.join(linux_dir, f"buildroot-{BUILDROOT_VER}")
    download_buildroot(linux_dir, buildroot_dir)
    write_defconfig(buildroot_dir)

    env = os.environ.copy()
    env["HOST_CFLAGS"] = (
        "-O2 -std=gnu11 -Wno-implicit-function-declaration -Wno-int-conversion -Wno-incompatible-pointer-types -Wno-return-type -Wno-error"
    )

    print("[Linux] Configuring Buildroot...")
    r = subprocess.run(
        ["make", "riscv_emu_defconfig"], cwd=buildroot_dir, env=env, check=False
    )
    if r.returncode != 0:
        return r.returncode

    print("[Linux] Building (this may take a while)...")
    nproc = os.cpu_count() or 4
    r = subprocess.run(["make", f"-j{nproc}"], cwd=buildroot_dir, env=env, check=False)
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
    ap.add_argument(
        "--rebuild",
        action="store_true",
        help="Rebuild the image even if one exists (after changing the package list)",
    )
    ap.add_argument(
        "--harts", type=int, default=8, help="Number of harts to boot (default 8)"
    )
    ap.add_argument(
        "--memory",
        choices=["ddr5", "dram"],
        default="ddr5",
        help="Memory subsystem: JEDEC DDR5 command timing or the row-buffer DRAM model",
    )
    ap.add_argument(
        "--speed-bin",
        default="5600B",
        help="DDR5 speed bin, 4800B or 5600B (default 5600B)",
    )
    ap.add_argument(
        "--interconnect",
        choices=sorted(INTERCONNECTS),
        default="mesh",
        help="Coherence interconnect between the cores (default mesh)",
    )
    args = ap.parse_args()

    if not args.no_build:
        if (
            args.rebuild
            or not os.path.exists(image_path)
            or not os.path.exists(os.path.join(out_dir, "fw_jump.bin"))
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

    cfg = presets.linux(
        args.harts,
        memory=args.memory,
        speed_bin=args.speed_bin,
        interconnect=args.interconnect,
        real_time=False,
    )
    print(
        f"[boot_linux] Booting {args.harts} hart(s), {args.interconnect} interconnect, "
        f"{args.memory}{' ' + args.speed_bin if args.memory == 'ddr5' else ''} memory..."
    )

    sim = Simulator(cfg, kernel=image_path, disk=disk_path)

    try:
        return sim.run(
            limit=10_000_000_000
        )  # Add progress = ... to this if it seems to hang.
    except RuntimeError as e:
        print(f"Simulation failed: {e}")
        return 1
    finally:
        print(sim.stats.summary())


if __name__ == "__main__":
    sys.exit(main())
