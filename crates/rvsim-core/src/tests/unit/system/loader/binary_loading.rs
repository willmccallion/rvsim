//! # Binary Loading Tests
//!
//! This module contains unit tests for the binary loading functionality,
//! including loading binaries from disk and setting up kernel boot configurations.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::Config;
use crate::isa::csr;
use crate::isa::privileged::PrivilegeMode;
use crate::isa::reg;
use crate::system::SystemState;
use crate::system::loader;
use crate::tests::support::harness::minimal_system;
use std::io::Write;
use tempfile::NamedTempFile;

/// Helper function to create a test CPU instance.
/// Helper function to create a temporary binary file for testing.
fn create_temp_binary(data: &[u8]) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(data).unwrap();
    file.flush().unwrap();
    file
}

#[test]
fn load_binary_returns_the_file_bytes_unchanged() {
    let test_data = vec![0x13, 0x00, 0x00, 0x00]; // RISC-V NOP instruction
    let temp_file = create_temp_binary(&test_data);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();
    assert_eq!(loaded_data, test_data);
}

#[test]
fn load_binary_of_an_empty_file_returns_no_bytes() {
    let temp_file = create_temp_binary(&[]);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();
    assert_eq!(loaded_data.len(), 0);
}

#[test]
fn load_binary_reads_a_1_kib_file_whole() {
    let test_data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
    let temp_file = create_temp_binary(&test_data);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();
    assert_eq!(loaded_data, test_data);
}

#[test]
fn a_missing_binary_is_an_error_naming_its_path() {
    let result = loader::load_binary("/nonexistent/path/that/cannot/exist.bin");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("/nonexistent/path/that/cannot/exist.bin"));
}

#[test]
fn without_firmware_the_hart_enters_an_mret_trampoline_to_the_kernel() {
    let mut state = minimal_system();
    let config = Config::minimal();

    // Setup without OpenSBI (default case when fw_jump.bin doesn't exist)
    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // Verify PC is set to RAM base
    assert_eq!(state.harts[0].pc, VirtAddr::new(config.system.ram_base.val()));

    // Verify privilege mode is Machine
    assert_eq!(state.harts[0].privilege, PrivilegeMode::Machine);

    // Verify MEPC is set to kernel load address
    let expected_mepc = config.system.ram_base.val() + config.system.kernel_offset;
    assert_eq!(state.core_ctx(0).csr_read(csr::MEPC), expected_mepc);

    // Verify registers are set up
    assert_eq!(state.harts[0].regs.read(reg::REG_A0), 0);
    assert_eq!(state.harts[0].regs.read(reg::REG_A1), config.system.ram_base.val() + 0x2200000);
}

#[test]
fn a_given_dtb_is_loaded_at_the_device_tree_offset() {
    let mut state = minimal_system();
    let config = Config::minimal();

    // Create a temporary DTB file
    let dtb_data = vec![0xd0, 0x0d, 0xfe, 0xed]; // DTB magic number
    let temp_dtb = create_temp_binary(&dtb_data);
    let dtb_path = temp_dtb.path().to_str().unwrap();

    let boot =
        loader::KernelBoot { dtb: Some(dtb_path.to_string()), ..loader::KernelBoot::default() };
    loader::setup_kernel_load(&mut state, &config, &boot).unwrap();

    // Verify DTB was loaded into memory at expected address
    let dtb_addr = config.system.ram_base.val() + 0x2200000;
    assert_eq!(state.uncore.kernel.memory.read(PhysAddr::new(dtb_addr), 1), Some(0xd0));
}

#[test]
fn an_explicit_firmware_is_loaded_at_ram_base_and_entered_in_machine_mode() {
    let mut state = minimal_system();
    let config = Config::minimal();
    let firmware = create_temp_binary(&[0x73, 0x00, 0x20, 0x30]);
    let boot = loader::KernelBoot {
        firmware: Some(firmware.path().to_str().unwrap().to_string()),
        ..loader::KernelBoot::default()
    };

    loader::setup_kernel_load(&mut state, &config, &boot).unwrap();

    let ram_base = config.system.ram_base;
    assert_eq!(state.uncore.kernel.memory.read(ram_base, 1), Some(0x73));
    assert_eq!(state.harts[0].pc, VirtAddr::new(ram_base.val()));
    assert_eq!(state.harts[0].privilege, PrivilegeMode::Machine);
    assert_eq!(state.harts[0].regs.read(reg::REG_A2), 0, "fw_jump takes no info struct");
}

#[test]
fn a_missing_explicit_firmware_is_an_error() {
    let mut state = minimal_system();
    let config = Config::minimal();
    let boot = loader::KernelBoot {
        firmware: Some("/nonexistent/fw_jump.bin".to_string()),
        ..loader::KernelBoot::default()
    };

    let error = loader::setup_kernel_load(&mut state, &config, &boot).expect_err("no firmware");

    assert!(error.to_string().contains("/nonexistent/fw_jump.bin"), "{error}");
}

/// A kernel the boot names must be there; booting the firmware alone is
/// only for when none was named and the default image is missing.
#[test]
fn a_missing_explicit_kernel_is_an_error() {
    let mut state = minimal_system();
    let config = Config::minimal();
    let firmware = create_temp_binary(&[0x73, 0x00, 0x20, 0x30]);
    let boot = loader::KernelBoot {
        kernel: Some("/nonexistent/Image".to_string()),
        firmware: Some(firmware.path().to_str().unwrap().to_string()),
        ..loader::KernelBoot::default()
    };

    let error = loader::setup_kernel_load(&mut state, &config, &boot).expect_err("no kernel");

    assert!(error.to_string().contains("/nonexistent/Image"), "{error}");
}

#[test]
fn without_firmware_a2_is_zero() {
    let mut state = minimal_system();
    let config = Config::minimal();

    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // a2 register should be 0
    assert_eq!(state.harts[0].regs.read(reg::REG_A2), 0);
}

#[test]
fn without_firmware_an_mret_is_placed_at_ram_base() {
    let mut state = minimal_system();
    let config = Config::minimal();

    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // MRET instruction (0x30200073) should be loaded at RAM base
    let ram_base = config.system.ram_base;
    assert_eq!(state.uncore.kernel.memory.read(ram_base, 4), Some(0x3020_0073));
}

#[test]
fn setting_up_a_kernel_load_twice_gives_the_same_pc() {
    let mut state = minimal_system();
    let config = Config::minimal();

    // First setup
    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();
    let pc_first = state.harts[0].pc;

    // Second setup (should overwrite)
    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();
    let pc_second = state.harts[0].pc;

    // Both should set the same PC
    assert_eq!(pc_first, pc_second);
}

#[test]
fn without_firmware_the_pc_follows_the_configured_ram_base() {
    // Test with different RAM base addresses
    let mut config1 = Config::minimal();
    config1.system.ram_base = PhysAddr::new(0x80000000);

    let mut config2 = Config::minimal();
    config2.system.ram_base = PhysAddr::new(0xA000_0000);

    let mut cpu1 = SystemState::build(&config1);
    loader::setup_kernel_load(&mut cpu1, &config1, &loader::KernelBoot::default()).unwrap();

    let mut cpu2 = SystemState::build(&config2);
    loader::setup_kernel_load(&mut cpu2, &config2, &loader::KernelBoot::default()).unwrap();

    // PC should match the respective RAM bases
    assert_eq!(cpu1.harts[0].pc, VirtAddr::new(0x80000000));
    assert_eq!(cpu2.harts[0].pc, VirtAddr::new(0xA000_0000));
}

#[test]
fn a_kernel_booted_by_firmware_is_loaded_at_the_configured_offset() {
    const KERNEL_OFFSET: u64 = 0x40_0000;
    const KERNEL_FIRST_BYTE: u8 = 0x13;
    let mut config = Config::minimal();
    config.system.kernel_offset = KERNEL_OFFSET;
    let mut state = SystemState::build(&config);
    let firmware = create_temp_binary(&[0x73, 0x00, 0x20, 0x30]);
    let kernel = create_temp_binary(&[KERNEL_FIRST_BYTE, 0, 0, 0]);
    let boot = loader::KernelBoot {
        firmware: Some(firmware.path().to_str().unwrap().to_string()),
        kernel: Some(kernel.path().to_str().unwrap().to_string()),
        ..loader::KernelBoot::default()
    };

    loader::setup_kernel_load(&mut state, &config, &boot).unwrap();

    let kernel_addr = PhysAddr::new(config.system.ram_base.val() + KERNEL_OFFSET);
    assert_eq!(state.uncore.kernel.memory.read(kernel_addr, 1), Some(u64::from(KERNEL_FIRST_BYTE)));
}
