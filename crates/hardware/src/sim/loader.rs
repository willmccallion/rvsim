//! Binary Loader and System Initialization (ELF + kernel/OpenSBI/DTB).

use crate::common::{PhysAddr, SimError};
use crate::config::Config;
use crate::core::arch::csr;
use crate::core::arch::mode::PrivilegeMode;
use crate::isa::abi;
use crate::isa::encoding::privileged as sys_ops;
use crate::sim::SimState;
use crate::soc::interconnect::Bus;
use object::{Object, ObjectSymbol};
use std::fs;

/// Loads a binary file from disk into a byte vector.
///
/// # Errors
///
/// Returns [`SimError::FileRead`] if the file cannot be opened or read.
pub fn load_binary(path: &str) -> Result<Vec<u8>, SimError> {
    fs::read(path).map_err(|source| SimError::FileRead { path: path.to_owned(), source })
}

/// The images a kernel boot loads.
#[derive(Clone, Debug, Default)]
pub struct KernelBoot {
    /// The kernel image; `software/linux/output/Image` when absent.
    pub kernel: Option<String>,
    /// An `OpenSBI` `fw_jump` firmware image; when absent, `fw_jump.bin` or
    /// `fw_dynamic.bin` under `software/linux/output`, if either exists.
    pub firmware: Option<String>,
    /// A device tree blob; generated from the config when absent.
    pub dtb: Option<String>,
}

/// Where the firmware comes from and how it hands over to the kernel.
enum Firmware {
    Jump(String),
    Dynamic(String),
}

impl Firmware {
    /// The boot's firmware, preferring `fw_jump` since it matches Spike's
    /// `fw_jump.elf` for log comparison.
    fn find(boot: &KernelBoot) -> Option<Self> {
        if let Some(path) = &boot.firmware {
            return Some(Self::Jump(path.clone()));
        }
        let jump = "software/linux/output/fw_jump.bin";
        let dynamic = "software/linux/output/fw_dynamic.bin";
        if fs::metadata(jump).is_ok() {
            Some(Self::Jump(jump.to_owned()))
        } else if fs::metadata(dynamic).is_ok() {
            Some(Self::Dynamic(dynamic.to_owned()))
        } else {
            None
        }
    }

    fn path(&self) -> &str {
        match self {
            Self::Jump(path) | Self::Dynamic(path) => path,
        }
    }
}

/// Sets up kernel loading: places `OpenSBI`, kernel image, and DTB in RAM and initializes CPU state.
///
/// If `OpenSBI` is found, loads it at `ram_base`, kernel at `ram_base + 0x200000`, DTB at `ram_base + 0x2200000`,
/// and sets PC to `OpenSBI` with a0/a1/a2 for DTB. Otherwise uses an MRET trampoline at `ram_base` and sets MEPC to kernel.
///
/// # Errors
///
/// Returns [`SimError::FileRead`] if any required binary file cannot be read from disk.
pub fn setup_kernel_load(
    state: &mut SimState,
    config: &Config,
    boot: &KernelBoot,
) -> Result<(), SimError> {
    let ram_base = config.system.ram_base;

    let opensbi_addr = ram_base;
    let kernel_addr = ram_base + 0x200000;
    let dtb_addr = ram_base + 0x2200000;

    if let Some(path) = &boot.dtb {
        let dtb_data = load_binary(path)?;
        state.load_binary_at(&dtb_data, PhysAddr::new(dtb_addr));
    } else {
        let dtb_data = crate::sim::dtb::generate_dtb(config);
        state.load_binary_at(&dtb_data, PhysAddr::new(dtb_addr));
    }

    if let Some(firmware) = Firmware::find(boot) {
        let sbi_data = load_binary(firmware.path())?;
        state.load_binary_at(&sbi_data, PhysAddr::new(opensbi_addr));

        let default_kernel_path = "software/linux/output/Image";
        let kernel_path = boot.kernel.as_deref().unwrap_or(default_kernel_path);

        if fs::metadata(kernel_path).is_ok() {
            let kernel_data = load_binary(kernel_path)?;
            state.load_binary_at(&kernel_data, PhysAddr::new(kernel_addr));
        } else {
            println!("[Loader] WARNING: Linux Image not found at {kernel_path}");
        }

        // Every hart starts in OpenSBI with its own id in a0; the firmware's
        // boot lottery picks the cold-boot hart and parks the rest.
        for hart in &mut state.harts {
            hart.pc = opensbi_addr;
            hart.privilege = PrivilegeMode::Machine;
            hart.regs.write(abi::REG_A0, u64::from(hart.hart_id.val()));
            hart.regs.write(abi::REG_A1, dtb_addr);
        }

        if matches!(firmware, Firmware::Dynamic(_)) {
            // fw_dynamic_info struct: magic, version, next_addr, next_mode,
            // options, boot_hart, next_arg1 (each u64 on rv64).
            const FW_DYNAMIC_INFO_MAGIC: u64 = 0x4942534f;
            const FW_DYNAMIC_INFO_VERSION: u64 = 2;
            const NEXT_MODE_S: u64 = 1;
            let info_addr = dtb_addr - 0x200;
            let fields: [u64; 7] = [
                FW_DYNAMIC_INFO_MAGIC,
                FW_DYNAMIC_INFO_VERSION,
                kernel_addr,
                NEXT_MODE_S,
                0,
                u64::MAX,
                dtb_addr,
            ];
            let mut info_bytes = Vec::with_capacity(56);
            for field in &fields {
                info_bytes.extend_from_slice(&field.to_le_bytes());
            }
            state.load_binary_at(&info_bytes, PhysAddr::new(info_addr));
            for hart in &mut state.harts {
                hart.regs.write(abi::REG_A2, info_addr);
            }
        } else {
            for hart in &mut state.harts {
                hart.regs.write(abi::REG_A2, 0);
            }
        }
    } else {
        let load_addr = ram_base + config.system.kernel_offset;

        state.load_binary_at(&sys_ops::MRET.to_le_bytes(), PhysAddr::new(ram_base));
        for core in 0..state.cores.len() {
            let mut ctx = state.core_ctx(core);
            ctx.hart.pc = ram_base;
            ctx.hart.privilege = PrivilegeMode::Machine;
            ctx.csr_write(csr::MEPC, load_addr);
            let hart_id = u64::from(ctx.hart.hart_id.val());
            ctx.hart.regs.write(abi::REG_A0, hart_id);
            ctx.hart.regs.write(abi::REG_A1, dtb_addr);
        }
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

    for segment in file.segments() {
        use object::ObjectSegment;
        let p_memsz = segment.size();
        if p_memsz == 0 {
            continue;
        }
        let paddr = segment.address();
        if let Ok(seg_data) = segment.data() {
            if !seg_data.is_empty() {
                bus.load_binary_at(seg_data, PhysAddr::new(paddr));
            }
            let p_filesz = seg_data.len() as u64;
            if p_memsz > p_filesz {
                let bss_start = paddr + p_filesz;
                let bss_size = (p_memsz - p_filesz) as usize;
                bus.load_binary_at(&vec![0u8; bss_size], PhysAddr::new(bss_start));
            }
        } else if p_memsz > 0 {
            bus.load_binary_at(&vec![0u8; p_memsz as usize], PhysAddr::new(paddr));
        }
    }

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
        let mut bus = Bus::new(8, 0, 1);
        let data = b"NOT AN ELF FILE";
        let result = try_load_elf(data, &mut bus);
        assert!(result.is_none());
    }

    #[test]
    fn test_try_load_elf_too_short() {
        let mut bus = Bus::new(8, 0, 1);
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
        let mut state = SimState::build(&config, "");

        setup_kernel_load(&mut state, &config, &KernelBoot::default()).unwrap();

        let ram_base = config.system.ram_base;
        let load_addr = ram_base + config.system.kernel_offset;

        assert_eq!(state.harts[0].pc, ram_base);
        assert_eq!(state.harts[0].privilege, PrivilegeMode::Machine);
        assert_eq!(state.core_ctx(0).csr_read(csr::MEPC), load_addr);
        assert_eq!(state.harts[0].regs.read(abi::REG_A0), 0);
        assert_eq!(state.harts[0].regs.read(abi::REG_A1), ram_base + 0x2200000);
    }
}
