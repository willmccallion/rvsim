//! A bare-metal image starts as under spike's boot ROM: every hart at the
//! entry in machine mode, its id in `a0`, the system's device tree in `a1`.

use crate::Simulator;
use crate::common::PhysAddr;
use crate::config::{Config, Console};
use crate::isa::privileged::PrivilegeMode;
use crate::isa::reg;
use crate::system::simulator::{DTB_RESERVE, ProcessArgs};

const RAM_BASE: u64 = 0x8000_0000;
const CODE_OFFSET: u64 = 0x1000;
const ET_EXEC: u16 = 2;
const EM_RISCV: u16 = 243;
const FDT_MAGIC: u32 = 0xd00d_feed;

/// A bare-metal executable holding `code` at `RAM_BASE + CODE_OFFSET`.
fn bare_elf(code: &[u32]) -> Vec<u8> {
    let entry = RAM_BASE + CODE_OFFSET;
    let mut image = Vec::new();
    image.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    image.extend_from_slice(&[0u8; 8]);
    image.extend_from_slice(&ET_EXEC.to_le_bytes());
    image.extend_from_slice(&EM_RISCV.to_le_bytes());
    image.extend_from_slice(&1u32.to_le_bytes());
    image.extend_from_slice(&entry.to_le_bytes());
    image.extend_from_slice(&64u64.to_le_bytes());
    image.extend_from_slice(&0u64.to_le_bytes());
    image.extend_from_slice(&0u32.to_le_bytes());
    image.extend_from_slice(&64u16.to_le_bytes());
    image.extend_from_slice(&56u16.to_le_bytes());
    image.extend_from_slice(&1u16.to_le_bytes());
    image.extend_from_slice(&[0u8; 6]);
    let file_size = CODE_OFFSET + (code.len() as u64) * 4;
    image.extend_from_slice(&1u32.to_le_bytes());
    image.extend_from_slice(&7u32.to_le_bytes());
    image.extend_from_slice(&0u64.to_le_bytes());
    image.extend_from_slice(&RAM_BASE.to_le_bytes());
    image.extend_from_slice(&RAM_BASE.to_le_bytes());
    image.extend_from_slice(&file_size.to_le_bytes());
    image.extend_from_slice(&file_size.to_le_bytes());
    image.extend_from_slice(&0x1000u64.to_le_bytes());
    image.resize(CODE_OFFSET as usize, 0);
    for word in code {
        image.extend_from_slice(&word.to_le_bytes());
    }
    image
}

#[test]
fn every_hart_starts_at_the_entry_with_its_id_and_the_device_tree() {
    let mut config = Config::default();
    config.system.hart_count = 3;
    config.system.console = Console::Quiet;
    let mut sim = Simulator::build(&config, "");

    sim.load_program(&bare_elf(&[0x0000_006f]), &ProcessArgs::default()).expect("the image loads");

    let dtb_addr = RAM_BASE + config.memory.ram_size as u64 - DTB_RESERVE;
    for (index, hart) in sim.state.harts.iter().enumerate() {
        assert_eq!(hart.pc, RAM_BASE + CODE_OFFSET, "hart {index}");
        assert_eq!(hart.privilege, PrivilegeMode::Machine, "hart {index}");
        assert_eq!(hart.regs.read(reg::REG_A0), index as u64, "hart {index}");
        assert_eq!(hart.regs.read(reg::REG_A1), dtb_addr, "hart {index}");
    }
    let magic = sim
        .state
        .uncore
        .memory
        .read_bytes(PhysAddr::new(dtb_addr), 4)
        .expect("the device tree is in RAM");
    assert_eq!(u32::from_be_bytes([magic[0], magic[1], magic[2], magic[3]]), FDT_MAGIC);
}
