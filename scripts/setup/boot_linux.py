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

from inspectre import SimConfig, Simulator

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

# Device tree matching sim: RAM 128MB @ 0x80000000, CLINT @ 0x02000000, UART @ 0x10000000, PLIC @ 0x0c000000.
# OpenSBI requires CLINT (timer) and PLIC with interrupts-extended to init irqchip.
SYSTEM_DTS = """/dts-v1/;

/ {
    #address-cells = <2>;
    #size-cells = <2>;
    compatible = "riscv-virtio";
    model = "riscv-virtio,qemu";

    chosen {
        bootargs = "root=/dev/vda rw console=ttyS0 earlycon=uart8250,mmio,0x10000000 rootwait";
        stdout-path = "/soc/uart@10000000";
    };

    cpus {
        #address-cells = <1>;
        #size-cells = <0>;
        timebase-frequency = <10000000>;

        cpu0: cpu@0 {
            device_type = "cpu";
            reg = <0>;
            status = "okay";
            compatible = "riscv";
            riscv,isa = "rv64imafdc";
            mmu-type = "riscv,sv39";

            cpu0_intc: interrupt-controller {
                #interrupt-cells = <1>;
                interrupt-controller;
                compatible = "riscv,cpu-intc";
            };
        };
    };

    memory@80000000 {
        device_type = "memory";
        reg = <0x0 0x80000000 0x0 0x10000000>;
    };

    soc {
        #address-cells = <2>;
        #size-cells = <2>;
        compatible = "simple-bus";
        ranges;

        clint: clint@2000000 {
            compatible = "riscv,clint0";
            reg = <0x0 0x02000000 0x0 0x10000>;
            interrupts-extended = <&cpu0_intc 3>, <&cpu0_intc 7>;
        };

        uart@10000000 {
            compatible = "ns16550a";
            reg = <0x0 0x10000000 0x0 0x100>;
            clock-frequency = <10000000>;
            status = "okay";
        };

        virtio_mmio@10001000 {
            compatible = "virtio,mmio";
            reg = <0x0 0x10001000 0x0 0x1000>;
            interrupt-parent = <&plic>;
            interrupts = <1>;
        };

        plic: interrupt-controller@c000000 {
            compatible = "riscv,plic0";
            reg = <0x0 0x0c000000 0x0 0x4000000>;
            #interrupt-cells = <1>;
            interrupt-controller;
            interrupts-extended = <&cpu0_intc 11>, <&cpu0_intc 9>;
            riscv,ndev = <0x35>;
        };
    };
};
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


def compile_dtb(linux_dir: str) -> int:
    """Write the DTS and compile it to DTB via dtc. Returns 0 on success."""
    dts_path = os.path.join(linux_dir, "system.dts")
    dtb_path = os.path.join(linux_dir, "system.dtb")
    with open(dts_path, "w") as f:
        f.write(SYSTEM_DTS)
    print("[Linux] Compiling device tree...")
    r = subprocess.run(
        ["dtc", "-I", "dts", "-O", "dtb", "-o", dtb_path, dts_path],
        cwd=linux_dir,
    )
    return r.returncode


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
    print("[Linux] Copied Image, disk.img, fw_jump.bin to", out_dir)

    rc = compile_dtb(linux_dir)
    if rc != 0:
        return rc
    print("[Linux] Build complete.")
    return 0


def optimized_config() -> SimConfig:
    """Machine config for Linux boot: 256MB RAM, full cache hierarchy, TAGE predictor."""
    c = SimConfig.default()

    # General
    c.general.trace_instructions = False
    c.general.start_pc = 0x80000000

    # System
    c.system.ram_base = 0x80000000
    c.system.uart_base = 0x10000000
    c.system.disk_base = 0x10001000
    c.system.clint_base = 0x02000000
    c.system.syscon_base = 0x00100000
    c.system.kernel_offset = 0x200000
    c.system.bus_width = 8
    c.system.bus_latency = 1
    c.system.clint_divider = 100

    # Memory
    c.memory.ram_size = 256 * 1024 * 1024
    c.memory.controller = "Simple"
    c.memory.row_miss_latency = 10
    c.memory.tlb_size = 64

    # L1 Instruction Cache
    c.cache.l1_i.enabled = True
    c.cache.l1_i.size_bytes = 65536
    c.cache.l1_i.line_bytes = 64
    c.cache.l1_i.ways = 8
    c.cache.l1_i.policy = "PLRU"
    c.cache.l1_i.latency = 1
    c.cache.l1_i.prefetcher = "NextLine"
    c.cache.l1_i.prefetch_degree = 2

    # L1 Data Cache
    c.cache.l1_d.enabled = True
    c.cache.l1_d.size_bytes = 65536
    c.cache.l1_d.line_bytes = 64
    c.cache.l1_d.ways = 8
    c.cache.l1_d.policy = "PLRU"
    c.cache.l1_d.latency = 1
    c.cache.l1_d.prefetcher = "Stride"
    c.cache.l1_d.prefetch_table_size = 128
    c.cache.l1_d.prefetch_degree = 2

    # L2 Cache
    c.cache.l2.enabled = True
    c.cache.l2.size_bytes = 1048576
    c.cache.l2.line_bytes = 64
    c.cache.l2.ways = 16
    c.cache.l2.policy = "PLRU"
    c.cache.l2.latency = 8
    c.cache.l2.prefetcher = "NextLine"
    c.cache.l2.prefetch_degree = 1

    # L3 Cache
    c.cache.l3.enabled = True
    c.cache.l3.size_bytes = 8 * 1024 * 1024
    c.cache.l3.line_bytes = 64
    c.cache.l3.ways = 16
    c.cache.l3.policy = "PLRU"
    c.cache.l3.latency = 28
    c.cache.l3.prefetcher = "None"

    # Pipeline
    c.pipeline.branch_predictor = "TAGE"
    c.pipeline.width = 1
    c.pipeline.btb_size = 4096
    c.pipeline.ras_size = 48

    # TAGE
    c.pipeline.tage.num_banks = 4
    c.pipeline.tage.table_size = 2048
    c.pipeline.tage.loop_table_size = 256
    c.pipeline.tage.reset_interval = 2000
    c.pipeline.tage.history_lengths = [5, 15, 44, 130]
    c.pipeline.tage.tag_widths = [9, 9, 10, 10]

    return c


def main():
    root = repo_root()
    linux_dir = os.path.join(root, "software", "linux")
    out_dir = os.path.join(linux_dir, "output")
    image_path = os.path.join(out_dir, "Image")
    disk_path = os.path.join(out_dir, "disk.img")
    dtb_path = os.path.join(linux_dir, "system.dtb")

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

    # Recompile DTB in case DTS changed (e.g. editing this script)
    if compile_dtb(linux_dir) != 0:
        return 1

    if not os.path.exists(disk_path):
        print("Error: disk image not found:", disk_path)
        return 1

    os.chdir(root)

    print("[boot_linux] Booting with Simulator (Optimized Config)...")

    sim = (
        Simulator()
        .with_config(optimized_config())
        .kernel(image_path)
        .disk(disk_path)
        .kernel_mode()
    )
    if os.path.isfile(dtb_path):
        sim.dtb(dtb_path)

    try:
        return sim.run()
    except Exception as e:
        print(f"Simulation failed: {e}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
