//! Binary Loader and System Initialization.
//!
//! This module provides utilities for loading binaries and setting up the initial CPU state. It performs:
//! 1. **Binary loading:** Reads kernel, firmware, or bare-metal binaries from disk into a byte buffer.
//! 2. **Kernel boot:** Loads OpenSBI, kernel image, and DTB at fixed addresses and sets PC and privilege.
//! 3. **Bare-metal fallback:** When no OpenSBI is present, sets up MRET trampoline and MEPC for direct boot.

use crate::config::Config;
use crate::core::Cpu;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::isa::abi;
use crate::isa::privileged::opcodes as sys_ops;
use std::fs;
use std::process;

/// Loads a binary file from disk into a byte vector.
///
/// Exits the process with an error message if the file cannot be read.
///
/// # Arguments
///
/// * `path` - Path to the binary file.
///
/// # Returns
///
/// The raw bytes of the file.
pub fn load_binary(path: &str) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| {
        eprintln!("\n[!] FATAL: Could not read file '{}': {}", path, e);
        process::exit(1);
    })
}

/// Sets up kernel loading: places OpenSBI, kernel image, and DTB in RAM and initializes CPU state.
///
/// If OpenSBI is found, loads it at `ram_base`, kernel at `ram_base + 0x200000`, DTB at `ram_base + 0x2200000`,
/// and sets PC to OpenSBI with a0/a1/a2 for DTB. Otherwise uses an MRET trampoline at `ram_base` and sets MEPC to kernel.
///
/// # Arguments
///
/// * `cpu` - Mutable reference to the CPU state.
/// * `config` - System configuration (RAM base, kernel offset).
/// * `_disk_path` - Reserved for disk path; currently unused.
/// * `dtb_path` - Optional path to the device tree blob; if provided, loaded at DTB address.
/// * `kernel_path_override` - Optional kernel image path; overrides default `software/linux/output/Image`.
pub fn setup_kernel_load(
    cpu: &mut Cpu,
    config: &Config,
    _disk_path: &str,
    dtb_path: Option<String>,
    kernel_path_override: Option<String>,
) {
    let ram_base = config.system.ram_base;

    let opensbi_addr = ram_base;
    let kernel_addr = ram_base + 0x200000;
    let dtb_addr = ram_base + 0x2200000;

    if let Some(path) = dtb_path {
        let dtb_data = load_binary(&path);
        cpu.bus.load_binary_at(&dtb_data, dtb_addr);
    }

    let sbi_path = "software/linux/output/fw_jump.bin";

    if fs::metadata(sbi_path).is_ok() {
        let sbi_data = load_binary(sbi_path);
        cpu.bus.load_binary_at(&sbi_data, opensbi_addr);

        let default_kernel_path = "software/linux/output/Image";
        let kernel_path = kernel_path_override
            .as_deref()
            .unwrap_or(default_kernel_path);

        if fs::metadata(kernel_path).is_ok() {
            let kernel_data = load_binary(kernel_path);
            cpu.bus.load_binary_at(&kernel_data, kernel_addr);
        } else {
            println!("[Loader] WARNING: Linux Image not found at {}", kernel_path);
        }

        cpu.pc = opensbi_addr;
        cpu.privilege = PrivilegeMode::Machine;
        cpu.regs.write(abi::REG_A0, 0);
        cpu.regs.write(abi::REG_A1, dtb_addr);
        cpu.regs.write(abi::REG_A2, 0);
    } else {
        let load_addr = ram_base + config.system.kernel_offset;

        cpu.bus
            .load_binary_at(&sys_ops::MRET.to_le_bytes(), ram_base);
        cpu.pc = ram_base;
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csr_write(csr::MEPC, load_addr);
        cpu.regs.write(abi::REG_A0, 0);
        cpu.regs.write(abi::REG_A1, dtb_addr);
    }
}
