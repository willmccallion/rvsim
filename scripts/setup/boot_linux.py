#!/usr/bin/env python3
"""
Download Buildroot, build Linux (kernel + rootfs + OpenSBI), then boot in the simulator.

Everything runs under software/linux: download, build, and artifacts (output/).
Run from repo root:

  sim script scripts/setup/boot_linux.py            # build if needed, then boot
  sim script scripts/setup/boot_linux.py --no-boot  # only download & build
  sim script scripts/setup/boot_linux.py --no-build # boot only (fail if no Image)

The default boot is the showcase system: four out-of-order cores from the
``fast`` preset, a MESI snoop-filter home agent over a 2-D mesh, and a
DDR5-5600 memory subsystem. ``--harts``, ``--memory``, ``--speed-bin`` and
``--interconnect`` trim it down.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tarfile
import urllib.request

from rvsim import (
    Cache,
    Coherence,
    HomeAgent,
    Interconnect,
    MemoryController,
    Simulator,
    presets,
)

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


INTERCONNECTS = {
    "crossbar": Interconnect.Crossbar,
    "ring": Interconnect.Ring,
    "mesh": Interconnect.Mesh,
    "torus": Interconnect.Torus,
    "hypercube": Interconnect.Hypercube,
}


def memory_controller(kind: str, speed_bin: str):
    """The DDR5 controller at ``speed_bin``, or the preset's row-buffer DRAM model."""
    if kind == "ddr5":
        return MemoryController.DDR5(speed_bin=speed_bin)
    return presets.fast().memory_controller


def config(
    hart_count: int = 4,
    memory: str = "ddr5",
    speed_bin: str = "5600B",
    interconnect: str = "mesh",
):
    """The Linux boot system.

    Starts from the ``fast`` preset and overrides the system addresses and
    memory-map settings that must match the device tree. ``hart_count``
    harts boot through OpenSBI's HSM into an SMP kernel; with more than
    one hart the private caches are kept coherent by a snoop-filter home
    agent over ``interconnect``. ``memory`` is ``ddr5`` (JEDEC command-level
    timing at ``speed_bin``) or ``dram`` (the preset's row-buffer model).
    """
    return presets.fast().replace(
        ram_size=256 * 1024 * 1024,
        ram_base=0x80000000,
        uart_base=0x10000000,
        disk_base=0x10001000,
        clint_base=0x02000000,
        syscon_base=0x00100000,
        kernel_offset=0x200000,
        clint_divider=1,
        hart_count=hart_count,
        coherence=Coherence(
            home_agent=HomeAgent.SnoopFilter(),
            interconnect=INTERCONNECTS[interconnect](),
        ),
        memory_controller=memory_controller(memory, speed_bin),
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
    ap.add_argument(
        "--harts", type=int, default=4, help="Number of harts to boot (default 4)"
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

    cfg = config(args.harts, args.memory, args.speed_bin, args.interconnect)
    print(
        f"[boot_linux] Booting {args.harts} hart(s), {args.interconnect} interconnect, "
        f"{args.memory}{' ' + args.speed_bin if args.memory == 'ddr5' else ''} memory..."
    )

    sim = Simulator(cfg, kernel=image_path, disk=disk_path)

    try:
        return sim.run(
            limit=10_000_000_000
        )  # Add progress = ... to this if it seems to hang.
    except Exception as e:
        print(f"Simulation failed: {e}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
