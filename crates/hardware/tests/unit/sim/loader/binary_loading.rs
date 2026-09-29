//! # Binary Loading Tests
//!
//! This module contains unit tests for the binary loading functionality,
//! including loading binaries from disk and setting up kernel boot configurations.

use rvsim_core::SimState;
use rvsim_core::config::Config;
use rvsim_core::core::arch::csr;
use rvsim_core::isa::privileged::PrivilegeMode;
use rvsim_core::isa::reg;
use rvsim_core::sim::loader;
use std::io::Write;
use tempfile::NamedTempFile;

/// Helper function to create a test CPU instance.
fn create_test_cpu() -> SimState {
    let config = Config::default();
    SimState::build(&config, "")
}

/// Helper function to create a temporary binary file for testing.
fn create_temp_binary(data: &[u8]) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(data).unwrap();
    file.flush().unwrap();
    file
}

#[test]
fn test_load_binary_success() {
    let test_data = vec![0x13, 0x00, 0x00, 0x00]; // RISC-V NOP instruction
    let temp_file = create_temp_binary(&test_data);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();
    assert_eq!(loaded_data, test_data);
}

#[test]
fn test_load_binary_empty_file() {
    let temp_file = create_temp_binary(&[]);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();
    assert_eq!(loaded_data.len(), 0);
}

#[test]
fn test_load_binary_large_file() {
    let test_data: Vec<u8> = (0..1024).map(|i| (i % 256) as u8).collect();
    let temp_file = create_temp_binary(&test_data);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();
    assert_eq!(loaded_data, test_data);
}

#[test]
fn test_load_binary_missing_file() {
    let result = loader::load_binary("/nonexistent/path/that/cannot/exist.bin");
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("/nonexistent/path/that/cannot/exist.bin"));
}

#[test]
fn test_setup_kernel_load_without_opensbi() {
    let mut state = create_test_cpu();
    let config = Config::default();

    // Setup without OpenSBI (default case when fw_jump.bin doesn't exist)
    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // Verify PC is set to RAM base
    assert_eq!(state.harts[0].pc, config.system.ram_base);

    // Verify privilege mode is Machine
    assert_eq!(state.harts[0].privilege, PrivilegeMode::Machine);

    // Verify MEPC is set to kernel load address
    let expected_mepc = config.system.ram_base + config.system.kernel_offset;
    assert_eq!(state.core_ctx(0).csr_read(csr::MEPC), expected_mepc);

    // Verify registers are set up
    assert_eq!(state.harts[0].regs.read(reg::REG_A0), 0);
    assert_eq!(state.harts[0].regs.read(reg::REG_A1), config.system.ram_base + 0x2200000);
}

#[test]
fn test_setup_kernel_load_dtb_address() {
    let mut state = create_test_cpu();
    let config = Config::default();

    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // DTB should be loaded at RAM base + 0x2200000
    let expected_dtb_addr = config.system.ram_base + 0x2200000;
    assert_eq!(state.harts[0].regs.read(reg::REG_A1), expected_dtb_addr);
}

#[test]
fn test_setup_kernel_load_with_dtb_file() {
    let mut state = create_test_cpu();
    let config = Config::default();

    // Create a temporary DTB file
    let dtb_data = vec![0xd0, 0x0d, 0xfe, 0xed]; // DTB magic number
    let temp_dtb = create_temp_binary(&dtb_data);
    let dtb_path = temp_dtb.path().to_str().unwrap();

    let boot =
        loader::KernelBoot { dtb: Some(dtb_path.to_string()), ..loader::KernelBoot::default() };
    loader::setup_kernel_load(&mut state, &config, &boot).unwrap();

    // Verify DTB was loaded into memory at expected address
    let dtb_addr = config.system.ram_base + 0x2200000;
    // Probe RAM directly via the bus's RamRegion: the bus's Handle defers
    // RAM reads to the memory controller, which is out of reach inside the
    // probe's local event queue. Loader-side data lives in DRAM unconditionally.
    let loaded_byte = unsafe { state.bus.ram_region().expect("ram region").ptr(dtb_addr).read() };
    assert_eq!(loaded_byte, 0xd0);
}

#[test]
fn an_explicit_firmware_is_loaded_at_ram_base_and_entered_in_machine_mode() {
    let mut state = create_test_cpu();
    let config = Config::default();
    let firmware = create_temp_binary(&[0x73, 0x00, 0x20, 0x30]);
    let boot = loader::KernelBoot {
        firmware: Some(firmware.path().to_str().unwrap().to_string()),
        ..loader::KernelBoot::default()
    };

    loader::setup_kernel_load(&mut state, &config, &boot).unwrap();

    let ram_base = config.system.ram_base;
    let first_byte = unsafe { state.bus.ram_region().expect("ram region").ptr(ram_base).read() };
    assert_eq!(first_byte, 0x73);
    assert_eq!(state.harts[0].pc, ram_base);
    assert_eq!(state.harts[0].privilege, PrivilegeMode::Machine);
    assert_eq!(state.harts[0].regs.read(reg::REG_A2), 0, "fw_jump takes no info struct");
}

#[test]
fn a_missing_explicit_firmware_is_an_error() {
    let mut state = create_test_cpu();
    let config = Config::default();
    let boot = loader::KernelBoot {
        firmware: Some("/nonexistent/fw_jump.bin".to_string()),
        ..loader::KernelBoot::default()
    };

    let result = loader::setup_kernel_load(&mut state, &config, &boot);

    assert!(result.is_err());
}

#[test]
fn test_setup_kernel_load_register_a2_is_zero() {
    let mut state = create_test_cpu();
    let config = Config::default();

    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // a2 register should be 0
    assert_eq!(state.harts[0].regs.read(reg::REG_A2), 0);
}

#[test]
fn test_setup_kernel_load_preserves_config() {
    let config = Config::default();
    let ram_base_before = config.system.ram_base;
    let kernel_offset_before = config.system.kernel_offset;

    let mut state = create_test_cpu();
    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // Config should not be modified
    assert_eq!(config.system.ram_base, ram_base_before);
    assert_eq!(config.system.kernel_offset, kernel_offset_before);
}

#[test]
fn test_setup_kernel_load_mret_instruction_at_ram_base() {
    let mut state = create_test_cpu();
    let config = Config::default();

    loader::setup_kernel_load(&mut state, &config, &loader::KernelBoot::default()).unwrap();

    // MRET instruction (0x30200073) should be loaded at RAM base
    let ram_base = config.system.ram_base;
    let instruction = unsafe {
        state.bus.ram_region().expect("ram region").ptr(ram_base).cast::<u32>().read_unaligned()
    };

    // MRET opcode is 0x30200073
    assert_eq!(instruction, 0x30200073);
}

#[test]
fn test_setup_kernel_load_multiple_calls() {
    let mut state = create_test_cpu();
    let config = Config::default();

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
fn test_setup_kernel_load_different_ram_bases() {
    // Test with different RAM base addresses
    let mut config1 = Config::default();
    config1.system.ram_base = 0x80000000;

    let mut config2 = Config::default();
    config2.system.ram_base = 0x90000000;

    let mut cpu1 = SimState::build(&config1, "");
    loader::setup_kernel_load(&mut cpu1, &config1, &loader::KernelBoot::default()).unwrap();

    let mut cpu2 = SimState::build(&config2, "");
    loader::setup_kernel_load(&mut cpu2, &config2, &loader::KernelBoot::default()).unwrap();

    // PC should match the respective RAM bases
    assert_eq!(cpu1.harts[0].pc, 0x80000000);
    assert_eq!(cpu2.harts[0].pc, 0x90000000);
}

#[test]
fn test_load_binary_content_integrity() {
    // Create a binary with specific pattern
    let test_data: Vec<u8> = (0..256).map(|i| i as u8).collect();
    let temp_file = create_temp_binary(&test_data);
    let path = temp_file.path().to_str().unwrap();

    let loaded_data = loader::load_binary(path).unwrap();

    // Verify every byte matches
    for (i, &byte) in loaded_data.iter().enumerate() {
        assert_eq!(byte, (i % 256) as u8, "Mismatch at byte {}", i);
    }
}
