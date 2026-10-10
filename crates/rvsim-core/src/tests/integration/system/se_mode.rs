//! SE mode: a static Linux ELF loads as a user process, its `ecall`s
//! are served as syscalls, and a fault ends it the way a signal would.

use crate::Simulator;
use crate::common::SimError;
use crate::config::{BackendKind, Config, Console};
use crate::system::simulator::{ProcessArgs, ProgramKind, StopAt, StopReason};
use crate::tests::support::builder::instruction::InstructionBuilder;

const ECALL: u32 = 0x0000_0073;
const SYS_EXIT: i32 = 93;
const SYS_BRK: i32 = 214;
const A0: u32 = 10;
const A1: u32 = 11;
const A7: u32 = 17;
const SP: u32 = 2;
const LOAD_ADDRESS: u64 = 0x1_0000;
const CODE_OFFSET: u64 = 0x80;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_RISCV: u16 = 243;
const RAM_BASE: u64 = 0x8000_0000;

/// A static riscv64 executable holding `code` in one readable, writable
/// and executable segment at `LOAD_ADDRESS`.
fn user_elf(code: &[u32], e_type: u16) -> Vec<u8> {
    let mut image = Vec::new();
    image.extend_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    image.extend_from_slice(&[0u8; 8]);
    image.extend_from_slice(&e_type.to_le_bytes());
    image.extend_from_slice(&EM_RISCV.to_le_bytes());
    image.extend_from_slice(&1u32.to_le_bytes());
    image.extend_from_slice(&(LOAD_ADDRESS + CODE_OFFSET).to_le_bytes());
    image.extend_from_slice(&64u64.to_le_bytes());
    image.extend_from_slice(&0u64.to_le_bytes());
    image.extend_from_slice(&0u32.to_le_bytes());
    image.extend_from_slice(&64u16.to_le_bytes());
    image.extend_from_slice(&56u16.to_le_bytes());
    image.extend_from_slice(&1u16.to_le_bytes());
    image.extend_from_slice(&[0u8; 6]);
    assert_eq!(image.len(), 64);

    let file_size = CODE_OFFSET + (code.len() as u64) * 4;
    image.extend_from_slice(&1u32.to_le_bytes());
    image.extend_from_slice(&7u32.to_le_bytes());
    image.extend_from_slice(&0u64.to_le_bytes());
    image.extend_from_slice(&LOAD_ADDRESS.to_le_bytes());
    image.extend_from_slice(&LOAD_ADDRESS.to_le_bytes());
    image.extend_from_slice(&file_size.to_le_bytes());
    image.extend_from_slice(&file_size.to_le_bytes());
    image.extend_from_slice(&0x1000u64.to_le_bytes());
    assert_eq!(image.len(), 120);

    image.resize(CODE_OFFSET as usize, 0);
    for word in code {
        image.extend_from_slice(&word.to_le_bytes());
    }
    image
}

fn config(backend: BackendKind) -> Config {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    config
}

/// Loads `code` as a process with `argv` and runs it to its exit.
fn run(backend: BackendKind, code: &[u32], argv: &[&str]) -> StopReason {
    let mut sim = Simulator::build(&config(backend), "");
    let args = ProcessArgs { argv: argv.iter().map(ToString::to_string).collect(), envp: vec![] };
    sim.load_process(&user_elf(code, ET_EXEC), &args).expect("the program loads");
    sim.run_to(&StopAt { cycles: Some(50_000), ..StopAt::default() }).expect("the program runs")
}

fn exit_with(a0: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().addi(A0, 0, a0).build(), i().addi(A7, 0, SYS_EXIT).build(), ECALL]
}

#[test]
fn a_user_program_exits_with_its_code() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &exit_with(42), &["prog"]);

        assert_eq!(stop, StopReason::Exited(42), "{backend:?}");
    }
}

/// `argc` sits at the stack pointer, as Linux's ELF loader leaves it.
#[test]
fn the_program_finds_its_arguments_on_the_stack() {
    let i = InstructionBuilder::new;
    let code = vec![i().ld(A0, SP, 0).build(), i().addi(A7, 0, SYS_EXIT).build(), ECALL];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog", "one", "two"]);

        assert_eq!(stop, StopReason::Exited(3), "{backend:?}");
    }
}

/// `brk` grows the heap, and the pages it adds can be written.
#[test]
fn brk_maps_the_heap_the_program_asks_for() {
    let i = InstructionBuilder::new;
    let code = vec![
        i().addi(A0, 0, 0).build(),
        i().addi(A7, 0, SYS_BRK).build(),
        ECALL,
        i().addi(A1, A0, 64).build(),
        i().addi(A0, A1, 0).build(),
        i().addi(A7, 0, SYS_BRK).build(),
        ECALL,
        i().sd(A0, A7, -8).build(),
        i().addi(A0, 0, 1).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(1), "{backend:?}");
    }
}

/// A load from an unmapped address is a page fault no kernel takes: the
/// program ends with SIGSEGV's exit status, 139.
#[test]
fn a_fault_ends_the_program_with_the_signals_exit_status() {
    let i = InstructionBuilder::new;
    let code = vec![i().lw(A0, 0, 0).build(), i().addi(A7, 0, SYS_EXIT).build(), ECALL];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(139), "{backend:?}");
    }
}

#[test]
fn a_position_independent_executable_is_refused() {
    let mut sim = Simulator::build(&config(BackendKind::InOrder), "");

    let err =
        sim.load_process(&user_elf(&exit_with(0), ET_DYN), &ProcessArgs::default()).unwrap_err();

    assert!(matches!(err, SimError::UnsupportedProgram { .. }), "{err}");
}

#[test]
fn a_user_program_is_told_from_a_bare_metal_image_by_its_entry() {
    let user = user_elf(&exit_with(0), ET_EXEC);

    assert_eq!(ProgramKind::of(&user, RAM_BASE).unwrap(), ProgramKind::Process);
    assert_eq!(ProgramKind::of(&user, LOAD_ADDRESS).unwrap(), ProgramKind::BareMetal);
}
