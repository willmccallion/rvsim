//! SE mode: a static Linux ELF loads as a user process, its `ecall`s
//! are served as syscalls, and a fault ends it the way a signal would.

use crate::Simulator;
use crate::common::SimError;
use crate::config::{BackendKind, Config, Console};
use crate::system::checkpoint::CheckpointError;
use crate::system::simulator::{ProcessArgs, ProgramKind, StopAt, StopReason};
use crate::tests::support::builder::instruction::InstructionBuilder;

const ECALL: u32 = 0x0000_0073;
const SYS_EXIT: i32 = 93;
const SYS_BRK: i32 = 214;
const SYS_OPENAT: i32 = 56;
const SYS_READ: i32 = 63;
const SYS_WRITE: i32 = 64;
const AT_FDCWD: i32 = -100;
const JAL_SELF: u32 = 0x0000_006f;
const T0: u32 = 5;
const T1: u32 = 6;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A7: u32 = 17;
const SP: u32 = 2;
const CYCLE: u32 = 0xC00;
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

/// A simulator on `config` with `code` loaded as a process with `argv`.
fn simulator_on(config: &Config, code: &[u32], argv: &[&str]) -> Simulator {
    let mut sim = Simulator::build(config, "");
    let args = ProcessArgs { argv: argv.iter().map(ToString::to_string).collect(), envp: vec![] };
    sim.load_process(&user_elf(code, ET_EXEC), &args).expect("the program loads");
    sim
}

/// A simulator with `code` loaded as a process with `argv`.
fn simulator(backend: BackendKind, code: &[u32], argv: &[&str]) -> Simulator {
    simulator_on(&config(backend), code, argv)
}

fn captured(backend: BackendKind) -> Config {
    let mut config = config(backend);
    config.system.console = Console::Captured;
    config
}

/// What a user program writes to its stdout reaches the simulator's
/// console, so a captured console hands it to the host.
#[test]
fn a_user_programs_output_reaches_the_console() {
    let i = InstructionBuilder::new;
    let mut code = vec![
        i().auipc(A1, 0).build(),
        i().addi(A1, A1, 8 * 4).build(),
        i().addi(A0, 0, 1).build(),
        i().addi(A2, 0, 3).build(),
        i().addi(A7, 0, SYS_WRITE).build(),
        ECALL,
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ];
    code.push(u32::from_le_bytes(*b"hi\n\0"));
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_on(&captured(backend), &code, &["prog"]);

        let stop = run_for(&mut sim, 50_000);

        assert_eq!(stop, StopReason::Exited(3), "{backend:?}");
        assert_eq!(sim.take_console_output(), b"hi\n", "{backend:?}");
    }
}

/// A user program's read of its stdin gets what the host typed into the
/// console.
#[test]
fn a_user_program_reads_what_the_host_typed() {
    let i = InstructionBuilder::new;
    let code = vec![
        i().addi(A0, 0, 0).build(),
        i().addi(A1, SP, -16).build(),
        i().addi(A2, 0, 1).build(),
        i().addi(A7, 0, SYS_READ).build(),
        ECALL,
        i().lbu(A0, SP, -16).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_on(&captured(backend), &code, &["prog"]);
        sim.send_console_input(b"7");

        let stop = run_for(&mut sim, 50_000);

        assert_eq!(stop, StopReason::Exited(u64::from(b'7')), "{backend:?}");
    }
}

fn run_for(sim: &mut Simulator, cycles: u64) -> StopReason {
    sim.run_to(&StopAt { cycles: Some(cycles), ..StopAt::default() }).expect("the program runs")
}

/// Loads `code` as a process with `argv` and runs it to its exit.
fn run(backend: BackendKind, code: &[u32], argv: &[&str]) -> StopReason {
    run_for(&mut simulator(backend, code, argv), 50_000)
}

/// Counts to 300, grows the heap with `brk` and writes to it, exits 9.
fn count_then_use_the_heap() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 0).build(),
        i().addi(T1, 0, 300).build(),
        i().addi(T0, T0, 1).build(),
        i().addi(A0, A0, 1).build(),
        i().blt(T0, T1, -8).build(),
        i().addi(A0, 0, 0).build(),
        i().addi(A7, 0, SYS_BRK).build(),
        ECALL,
        i().addi(A1, A0, 64).build(),
        i().addi(A0, A1, 0).build(),
        i().addi(A7, 0, SYS_BRK).build(),
        ECALL,
        i().sd(A0, A7, -8).build(),
        i().addi(A0, 0, 9).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ]
}

/// A checkpoint taken in the loop restores the process: the run finishes
/// as the uninterrupted one, `brk` still maps the heap, and the same
/// instructions retire.
#[test]
fn a_checkpoint_resumes_a_user_program() {
    let code = count_then_use_the_heap();
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut whole = simulator(backend, &code, &["prog"]);
        let expected = run_for(&mut whole, 50_000);
        let mut first = simulator(backend, &code, &["prog"]);
        run_for(&mut first, 200);
        let mut saved = Vec::new();
        first.save_checkpoint(&mut saved).expect("a user program's checkpoint saves");

        let mut second = simulator(backend, &code, &["prog"]);
        second.restore_checkpoint(&mut saved.as_slice()).expect("the checkpoint restores");
        let outcome = run_for(&mut second, 50_000);

        assert_eq!(expected, StopReason::Exited(9), "{backend:?}");
        assert_eq!(outcome, expected, "{backend:?}");
        assert_eq!(second.instructions_retired(), whole.instructions_retired(), "{backend:?}");
    }
}

/// With a host file open, a checkpoint is refused: the descriptor would
/// not come back.
#[test]
fn a_checkpoint_is_refused_while_a_host_file_is_open() {
    let i = InstructionBuilder::new;
    let path_offset = 8 * 4;
    let mut code = vec![
        i().auipc(A1, 0).build(),
        i().addi(A1, A1, path_offset).build(),
        i().addi(A0, 0, AT_FDCWD).build(),
        i().addi(A2, 0, 0).build(),
        i().addi(A3, 0, 0).build(),
        i().addi(A7, 0, SYS_OPENAT).build(),
        ECALL,
        JAL_SELF,
    ];
    code.extend(
        b"/dev/null\0\0\0"
            .chunks(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]])),
    );
    let mut sim = simulator(BackendKind::InOrder, &code, &["prog"]);
    run_for(&mut sim, 2_000);

    let refused = sim.save_checkpoint(&mut Vec::new());

    assert!(matches!(refused, Err(CheckpointError::Unsupported(_))), "{refused:?}");
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

/// A user program reads the cycle counter without trapping, as it can
/// under gem5 and qemu; the loader opens the counters to user mode.
#[test]
fn a_user_program_reads_the_cycle_counter() {
    let i = InstructionBuilder::new;
    let code = vec![
        i().csrrs(A0, CYCLE, 0).build(),
        i().sltu(A0, 0, A0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(1), "{backend:?}");
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
