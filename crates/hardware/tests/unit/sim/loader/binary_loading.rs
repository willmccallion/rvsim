//! # Binary Loading Tests
//!
//! This module contains unit tests for the binary loading functionality,
//! including loading binaries from disk and setting up kernel boot configurations.

use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::Cpu;
use rvsim_core::core::arch::csr;
use rvsim_core::core::arch::mode::PrivilegeMode;
use rvsim_core::isa::abi;
use rvsim_core::sim::loader;
use std::io::Write;
use tempfile::NamedTempFile;

/// Helper function to create a test CPU instance.
fn create_test_cpu() -> Cpu {
    let config = Config::default();
    let system = rvsim_core::soc::System::new(&config, "");
    Cpu::new(system, &config)
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
    let mut cpu = create_test_cpu();
    let config = Config::default();

    // Setup without OpenSBI (default case when fw_jump.bin doesn't exist)
    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();

    // Verify PC is set to RAM base
    assert_eq!(cpu.pc, config.system.ram_base);

    // Verify privilege mode is Machine
    assert_eq!(cpu.privilege, PrivilegeMode::Machine);

    // Verify MEPC is set to kernel load address
    let expected_mepc = config.system.ram_base + config.system.kernel_offset;
    assert_eq!(cpu.csr_read(csr::MEPC), expected_mepc);

    // Verify registers are set up
    assert_eq!(cpu.regs.read(abi::REG_A0), 0);
    assert_eq!(cpu.regs.read(abi::REG_A1), config.system.ram_base + 0x2200000);
}

#[test]
fn test_setup_kernel_load_dtb_address() {
    let mut cpu = create_test_cpu();
    let config = Config::default();

    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();

    // DTB should be loaded at RAM base + 0x2200000
    let expected_dtb_addr = config.system.ram_base + 0x2200000;
    assert_eq!(cpu.regs.read(abi::REG_A1), expected_dtb_addr);
}

#[test]
fn test_setup_kernel_load_with_dtb_file() {
    let mut cpu = create_test_cpu();
    let config = Config::default();

    // Create a temporary DTB file
    let dtb_data = vec![0xd0, 0x0d, 0xfe, 0xed]; // DTB magic number
    let temp_dtb = create_temp_binary(&dtb_data);
    let dtb_path = temp_dtb.path().to_str().unwrap();

    loader::setup_kernel_load(&mut cpu, &config, "", Some(dtb_path.to_string()), None).unwrap();

    // Verify DTB was loaded into memory at expected address
    let dtb_addr = config.system.ram_base + 0x2200000;
    let loaded_byte = cpu.bus.bus.read_u8(PhysAddr::new(dtb_addr));
    assert_eq!(loaded_byte, 0xd0);
}

#[test]
fn test_setup_kernel_load_register_a2_is_zero() {
    let mut cpu = create_test_cpu();
    let config = Config::default();

    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();

    // a2 register should be 0
    assert_eq!(cpu.regs.read(abi::REG_A2), 0);
}

#[test]
fn test_setup_kernel_load_preserves_config() {
    let config = Config::default();
    let ram_base_before = config.system.ram_base;
    let kernel_offset_before = config.system.kernel_offset;

    let mut cpu = create_test_cpu();
    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();

    // Config should not be modified
    assert_eq!(config.system.ram_base, ram_base_before);
    assert_eq!(config.system.kernel_offset, kernel_offset_before);
}

#[test]
fn test_setup_kernel_load_mret_instruction_at_ram_base() {
    let mut cpu = create_test_cpu();
    let config = Config::default();

    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();

    // MRET instruction (0x30200073) should be loaded at RAM base
    let ram_base = config.system.ram_base;
    let instruction = cpu.bus.bus.read_u32(PhysAddr::new(ram_base));

    // MRET opcode is 0x30200073
    assert_eq!(instruction, 0x30200073);
}

#[test]
fn test_setup_kernel_load_multiple_calls() {
    let mut cpu = create_test_cpu();
    let config = Config::default();

    // First setup
    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();
    let pc_first = cpu.pc;

    // Second setup (should overwrite)
    loader::setup_kernel_load(&mut cpu, &config, "", None, None).unwrap();
    let pc_second = cpu.pc;

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

    let system1 = rvsim_core::soc::System::new(&config1, "");
    let mut cpu1 = Cpu::new(system1, &config1);
    loader::setup_kernel_load(&mut cpu1, &config1, "", None, None).unwrap();

    let system2 = rvsim_core::soc::System::new(&config2, "");
    let mut cpu2 = Cpu::new(system2, &config2);
    loader::setup_kernel_load(&mut cpu2, &config2, "", None, None).unwrap();

    // PC should match the respective RAM bases
    assert_eq!(cpu1.pc, 0x80000000);
    assert_eq!(cpu2.pc, 0x90000000);
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
