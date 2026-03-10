//! Binary Loader and System Initialization.
//!
//! This module provides utilities for loading binaries and setting up the initial CPU state. It performs:
//! 1. **Binary loading:** Reads kernel, firmware, or bare-metal binaries from disk into a byte buffer.
//! 2. **Kernel boot:** Loads `OpenSBI`, kernel image, and DTB at fixed addresses and sets PC and privilege.
//! 3. **Bare-metal fallback:** When no `OpenSBI` is present, sets up MRET trampoline and MEPC for direct boot.

use crate::common::{PhysAddr, SimError};
use crate::config::Config;
use crate::core::Cpu;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::isa::abi;
use crate::isa::privileged::opcodes as sys_ops;
use crate::soc::interconnect::Bus;
use object::{Object, ObjectSymbol};
use std::fs;

/// Loads a binary file from disk into a byte vector.
///
/// # Arguments
///
/// * `path` - Path to the binary file.
///
/// # Errors
///
/// Returns [`SimError::FileRead`] if the file cannot be opened or read.
pub fn load_binary(path: &str) -> Result<Vec<u8>, SimError> {
    fs::read(path).map_err(|source| SimError::FileRead { path: path.to_owned(), source })
}

/// Sets up kernel loading: places `OpenSBI`, kernel image, and DTB in RAM and initializes CPU state.
///
/// If `OpenSBI` is found, loads it at `ram_base`, kernel at `ram_base + 0x200000`, DTB at `ram_base + 0x2200000`,
/// and sets PC to `OpenSBI` with a0/a1/a2 for DTB. Otherwise uses an MRET trampoline at `ram_base` and sets MEPC to kernel.
///
/// # Arguments
///
/// * `cpu` - Mutable reference to the CPU state.
/// * `config` - System configuration (RAM base, kernel offset).
/// * `_disk_path` - Reserved for disk path; currently unused.
/// * `dtb_path` - Optional path to the device tree blob; if provided, loaded at DTB address.
/// * `kernel_path_override` - Optional kernel image path; overrides default `software/linux/output/Image`.
///
/// # Errors
///
/// Returns [`SimError::FileRead`] if any required binary file cannot be read from disk.
#[allow(clippy::needless_pass_by_value)]
pub fn setup_kernel_load(
    cpu: &mut Cpu,
    config: &Config,
    _disk_path: &str,
    dtb_path: Option<String>,
    kernel_path_override: Option<String>,
) -> Result<(), SimError> {
    let ram_base = config.system.ram_base;

    let opensbi_addr = ram_base;
    let kernel_addr = ram_base + 0x200000;
    let dtb_addr = ram_base + 0x2200000;

    if let Some(path) = dtb_path {
        let dtb_data = load_binary(&path)?;
        cpu.bus.load_binary_at(&dtb_data, PhysAddr::new(dtb_addr));
    } else {
        // Generate DTB from SoC config when no external DTB is provided.
        let dtb_data = crate::sim::dtb::generate_dtb(config);
        cpu.bus.load_binary_at(&dtb_data, PhysAddr::new(dtb_addr));
    }

    // Prefer fw_jump.bin (matches spike's fw_jump.elf for log comparison)
    // over fw_dynamic.bin (which requires extra fw_dynamic_info setup).
    let sbi_jump_path = "software/linux/output/fw_jump.bin";
    let sbi_dynamic_path = "software/linux/output/fw_dynamic.bin";
    let sbi_path =
        if fs::metadata(sbi_jump_path).is_ok() { sbi_jump_path } else { sbi_dynamic_path };

    if fs::metadata(sbi_path).is_ok() {
        let sbi_data = load_binary(sbi_path)?;
        cpu.bus.load_binary_at(&sbi_data, PhysAddr::new(opensbi_addr));

        let default_kernel_path = "software/linux/output/Image";
        let kernel_path = kernel_path_override.as_deref().unwrap_or(default_kernel_path);

        if fs::metadata(kernel_path).is_ok() {
            let kernel_data = load_binary(kernel_path)?;
            cpu.bus.load_binary_at(&kernel_data, PhysAddr::new(kernel_addr));
        } else {
            println!("[Loader] WARNING: Linux Image not found at {kernel_path}");
        }

        cpu.pc = opensbi_addr;
        cpu.privilege = PrivilegeMode::Machine;
        cpu.regs.write(abi::REG_A0, 0); // hartid
        cpu.regs.write(abi::REG_A1, dtb_addr); // FDT address

        if sbi_path == sbi_dynamic_path {
            // Build the fw_dynamic_info struct in RAM and pass its address in a2.
            // struct fw_dynamic_info (each field is u64 on rv64):
            //   magic:     0x4942534f ("OSBI")
            //   version:   2
            //   next_addr: kernel entry point
            //   next_mode: 1 (Supervisor)
            //   options:   0
            //   boot_hart: -1 (any hart)
            //   next_arg1: DTB address passed to next stage (kernel)
            const FW_DYNAMIC_INFO_MAGIC: u64 = 0x4942534f;
            const FW_DYNAMIC_INFO_VERSION: u64 = 2;
            const NEXT_MODE_S: u64 = 1;
            let info_addr = dtb_addr - 0x200;
            let fields: [u64; 7] = [
                FW_DYNAMIC_INFO_MAGIC,
                FW_DYNAMIC_INFO_VERSION,
                kernel_addr,
                NEXT_MODE_S,
                0,        // options
                u64::MAX, // boot_hart = any
                dtb_addr, // next_arg1 = DTB for kernel
            ];
            let mut info_bytes = Vec::with_capacity(56);
            for field in &fields {
                info_bytes.extend_from_slice(&field.to_le_bytes());
            }
            cpu.bus.load_binary_at(&info_bytes, PhysAddr::new(info_addr));
            cpu.regs.write(abi::REG_A2, info_addr);
        } else {
            cpu.regs.write(abi::REG_A2, 0);
        }
    } else {
        let load_addr = ram_base + config.system.kernel_offset;

        cpu.bus.load_binary_at(&sys_ops::MRET.to_le_bytes(), PhysAddr::new(ram_base));
        cpu.pc = ram_base;
        cpu.privilege = PrivilegeMode::Machine;
        cpu.csr_write(csr::MEPC, load_addr);
        cpu.regs.write(abi::REG_A0, 0);
        cpu.regs.write(abi::REG_A1, dtb_addr);
    }

    Ok(())
}

/// Result of loading an ELF file.
#[derive(Debug)]
pub struct ElfLoadResult {
    /// Entry point address from the ELF header.
    pub entry: u64,
    /// Address of the `tohost` symbol, if present.
    pub tohost_addr: Option<u64>,
}

/// Attempts to load an ELF file into memory via the bus.
///
/// If the file starts with the ELF magic (`\x7fELF`), parses the ELF,
/// loads all `PT_LOAD` segments, and extracts the `tohost` symbol address.
/// Returns `None` if the data is not a valid ELF.
pub fn try_load_elf(data: &[u8], bus: &mut Bus) -> Option<ElfLoadResult> {
    if data.len() < 4 || &data[..4] != b"\x7fELF" {
        return None;
    }

    let file = object::File::parse(data).ok()?;
    let entry = file.entry();

    // Load ELF segments into memory
    for segment in file.segments() {
        use object::ObjectSegment;
        // We only care about loadable segments (PT_LOAD flags)
        let p_memsz = segment.size();
        if p_memsz == 0 {
            continue;
        }
        let paddr = segment.address();
        if let Ok(seg_data) = segment.data() {
            if !seg_data.is_empty() {
                bus.load_binary_at(seg_data, PhysAddr::new(paddr));
            }
            // Zero-fill the BSS gap (p_memsz > p_filesz)
            let p_filesz = seg_data.len() as u64;
            if p_memsz > p_filesz {
                let bss_start = paddr + p_filesz;
                let bss_size = (p_memsz - p_filesz) as usize;
                bus.load_binary_at(&vec![0u8; bss_size], PhysAddr::new(bss_start));
            }
        } else if p_memsz > 0 {
            // No file data but memsz > 0: zero-fill the entire region
            bus.load_binary_at(&vec![0u8; p_memsz as usize], PhysAddr::new(paddr));
        }
    }

    // Find tohost symbol
    let tohost_addr = file.symbols().find(|s| s.name() == Ok("tohost")).map(|s| s.address());

    Some(ElfLoadResult { entry, tohost_addr })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, unused_results)]
mod tests {
    use super::*;
    use crate::soc::interconnect::Bus;
    use std::io::Write;

    #[test]
    fn test_try_load_elf_invalid() {
        let mut bus = Bus::new(8, 0);
        let data = b"NOT AN ELF FILE";
        let result = try_load_elf(data, &mut bus);
        assert!(result.is_none());
    }

    #[test]
    fn test_try_load_elf_too_short() {
        let mut bus = Bus::new(8, 0);
        let data = b"EL";
        let result = try_load_elf(data, &mut bus);
        assert!(result.is_none());
    }

    #[test]
    fn test_load_binary_success() {
        let mut temp_file = tempfile::NamedTempFile::new().unwrap();
        temp_file.write_all(b"Hello World").unwrap();

        let path = temp_file.path().to_str().unwrap();
        let data = load_binary(path).unwrap();

        assert_eq!(data, b"Hello World");
    }

    #[test]
    fn test_load_binary_missing_file() {
        let result = load_binary("/nonexistent/path/that/cannot/exist.bin");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("/nonexistent/path/that/cannot/exist.bin"));
    }

    #[test]
    fn test_setup_kernel_load_fallback() {
        let config = Config::default();
        // Create an empty system.
        let system = crate::soc::builder::System::new(&config, "");
        let mut cpu = Cpu::new(system, &config);

        // We assume the OpenSBI files do not exist in the test environment,
        // so it will hit the fallback path.
        setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();

        let ram_base = config.system.ram_base;
        let load_addr = ram_base + config.system.kernel_offset;

        assert_eq!(cpu.pc, ram_base);
        assert_eq!(cpu.privilege, PrivilegeMode::Machine);
        assert_eq!(cpu.csr_read(csr::MEPC), load_addr);
        assert_eq!(cpu.regs.read(abi::REG_A0), 0);
        assert_eq!(cpu.regs.read(abi::REG_A1), ram_base + 0x2200000); // dtb_addr
    }
}
