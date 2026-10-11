//! SE mode: a static Linux ELF loads as a user process, its `ecall`s
//! are served as syscalls, and a fault ends it the way a signal would.

use crate::Simulator;
use crate::common::SimError;
use crate::config::{BackendKind, Config, Console};
use crate::isa::op::pseudo::PseudoOp;
use crate::system::checkpoint::CheckpointError;
use crate::system::process::loader::PIE_BASE;
use crate::system::simulator::{ProcessArgs, ProgramImage, ProgramKind, StopAt, StopReason};
use crate::tests::support::builder::instruction::InstructionBuilder;

const ECALL: u32 = 0x0000_0073;
const SYS_EXIT: i32 = 93;
const SYS_BRK: i32 = 214;
const SYS_OPENAT: i32 = 56;
const SYS_READ: i32 = 63;
const SYS_IOCTL: i32 = 29;
const SYS_RISCV_HWPROBE: i32 = 258;
const SYS_GETPID: i32 = 172;
const SYS_GETTID: i32 = 178;
const A4: u32 = 14;
const M5_FAIL: u32 = 0x22;
const M5_DUMP_STATS: u32 = 0x41;
const SYS_WRITE: i32 = 64;
const AT_FDCWD: i32 = -100;
const JAL_SELF: u32 = 0x0000_006f;
/// `vsetvli x0, x0, e8`: a vector instruction with no effect on the test.
const VSETVLI_ZERO: u32 = 0x0000_7057;
const T0: u32 = 5;
const T1: u32 = 6;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const A3: u32 = 13;
const A7: u32 = 17;
const A5: u32 = 15;
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
pub(super) fn user_elf(code: &[u32], e_type: u16) -> Vec<u8> {
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

/// A private mapping of a file is the file's bytes, read-only pages
/// included: the program reads the first byte of a mapping it may not
/// write.
#[test]
fn a_read_only_file_mapping_holds_the_files_bytes() {
    const SYS_MMAP: i32 = 222;
    const PROT_READ: i32 = 1;
    const MAP_PRIVATE: i32 = 2;
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("m");
    std::fs::write(&path, b"Z").expect("the file is written");
    let i = InstructionBuilder::new;
    let mut code = vec![
        i().auipc(A1, 0).build(),
        i().addi(A1, A1, 18 * 4).build(),
        i().addi(A0, 0, AT_FDCWD).build(),
        i().addi(A2, 0, 0).build(),
        i().addi(A3, 0, 0).build(),
        i().addi(A7, 0, SYS_OPENAT).build(),
        ECALL,
        i().addi(A4, A0, 0).build(),
        i().addi(A0, 0, 0).build(),
        i().lui(A1, 1).build(),
        i().addi(A2, 0, PROT_READ).build(),
        i().addi(A3, 0, MAP_PRIVATE).build(),
        i().addi(A5, 0, 0).build(),
        i().addi(A7, 0, SYS_MMAP).build(),
        ECALL,
        i().lbu(A0, A0, 0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ];
    let mut name = path.to_str().expect("a UTF-8 path").as_bytes().to_vec();
    name.resize(name.len().next_multiple_of(4) + 4, 0);
    code.extend(
        name.chunks(4).map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]])),
    );
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(u64::from(b'Z')), "{backend:?}");
    }
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

/// The console answers `TCGETS2`, the `tcgetattr` of glibc 2.42, so
/// `isatty` holds and stdio line-buffers what the program prints.
#[test]
fn the_console_is_a_terminal_to_tcgets2() {
    let i = InstructionBuilder::new;
    let code = vec![
        i().addi(A0, 0, 1).build(),
        i().lui(A1, 0x802c5).build(),
        i().addi(A1, A1, 0x42a).build(),
        i().addi(A2, SP, -64).build(),
        i().addi(A7, 0, SYS_IOCTL).build(),
        ECALL,
        i().addi(A0, A0, 1).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(1), "{backend:?}");
    }
}

/// gem5's `m5_fail` ends the run with its code from a user program too.
#[test]
fn a_pseudo_op_ends_a_user_program_with_its_code() {
    let i = InstructionBuilder::new;
    let code = vec![i().addi(A1, 0, 7).build(), PseudoOp::encode(M5_FAIL)];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(7), "{backend:?}");
    }
}

/// A stats dump asked for with a pseudo-operation carries `a0` as its label.
#[test]
fn a_pseudo_op_dumps_the_stats_with_its_label() {
    let i = InstructionBuilder::new;
    let mut code = vec![i().addi(A0, 0, 3).build(), PseudoOp::encode(M5_DUMP_STATS)];
    code.extend(exit_with(0));
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator(backend, &code, &["prog"]);

        let stop = run_for(&mut sim, 50_000);

        assert_eq!(stop, StopReason::Exited(0), "{backend:?}");
        let labels: Vec<u64> = sim.state.stats_dumps.iter().map(|dump| dump.label).collect();
        assert_eq!(labels, vec![3], "{backend:?}");
    }
}

/// A user program's vector instructions run: glibc's string routines use
/// them when the ISA has the extension, and Linux enables the unit.
#[test]
fn a_user_program_uses_the_vector_unit() {
    let mut code = vec![VSETVLI_ZERO];
    code.extend(exit_with(0));
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(0), "{backend:?}");
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

/// A static position-independent executable loads at the PIE base, its
/// entry moved with it, and runs.
#[test]
fn a_static_position_independent_executable_loads_and_runs() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = Simulator::build(&config(backend), "");

        sim.load_process(&user_elf(&exit_with(3), ET_DYN), &ProcessArgs::default())
            .expect("a static PIE loads");

        assert_eq!(
            sim.state.harts[0].pc,
            PIE_BASE.val() + LOAD_ADDRESS + CODE_OFFSET,
            "{backend:?}"
        );
        assert_eq!(run_for(&mut sim, 50_000), StopReason::Exited(3), "{backend:?}");
    }
}

#[test]
fn a_user_program_is_told_from_a_bare_metal_image_by_its_entry() {
    let user = user_elf(&exit_with(0), ET_EXEC);

    assert_eq!(ProgramKind::of(&user, RAM_BASE).unwrap(), ProgramKind::Process);
    assert_eq!(ProgramKind::of(&user, LOAD_ADDRESS).unwrap(), ProgramKind::BareMetal);
}

/// One `riscv_hwprobe` pair with `key` on the stack, probed, then its
/// value (or its key, with `exit_with_key`) as the exit code.
fn hwprobe_program(key: i32, exit_with_key: bool) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, key).build(),
        i().sd(SP, T0, -16).build(),
        i().sd(SP, 0, -8).build(),
        i().addi(A0, SP, -16).build(),
        i().addi(A1, 0, 1).build(),
        i().addi(A2, 0, 0).build(),
        i().addi(A3, 0, 0).build(),
        i().addi(A4, 0, 0).build(),
        i().addi(A7, 0, SYS_RISCV_HWPROBE).build(),
        ECALL,
        i().ld(A0, SP, if exit_with_key { -16 } else { -8 }).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ]
}

/// `riscv_hwprobe` reports the extensions a 6.6 kernel knows from the
/// hart's ISA string: all six with the default ISA, C alone for `rv64imac`.
#[test]
fn hwprobe_reports_the_extensions_the_isa_string_names() {
    const FD_C_V_ZBA_ZBB_ZBS: u64 = 0b11_1111;
    const C: u64 = 0b10;
    let mut imac = config(BackendKind::InOrder);
    imac.isa.string = Some("rv64imac".to_owned());
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let full = run(backend, &hwprobe_program(4, false), &["prog"]);
        let mut sim = simulator_on(&imac, &hwprobe_program(4, false), &["prog"]);
        let narrow = run_for(&mut sim, 50_000);

        assert_eq!(full, StopReason::Exited(FD_C_V_ZBA_ZBB_ZBS), "{backend:?}");
        assert_eq!(narrow, StopReason::Exited(C), "{backend:?}");
    }
}

/// Misaligned accesses are fast unless the hart traps on them, and a key
/// the kernel does not know comes back as -1.
#[test]
fn hwprobe_reports_misaligned_access_speed_and_unknown_keys() {
    const FAST: u64 = 3;
    const UNSUPPORTED: u64 = 4;
    let mut trapping = config(BackendKind::OutOfOrder);
    trapping.memory.misaligned_access_trap = true;
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        assert_eq!(run(backend, &hwprobe_program(5, false), &["prog"]), StopReason::Exited(FAST));
        assert_eq!(run(backend, &hwprobe_program(99, true), &["prog"]), StopReason::Exited(0xff));
    }
    let mut sim = simulator_on(&trapping, &hwprobe_program(5, false), &["prog"]);
    assert_eq!(run_for(&mut sim, 50_000), StopReason::Exited(UNSUPPORTED));
}

/// A simulator with one hart per program running `programs`.
fn simulator_with_programs(backend: BackendKind, programs: &[&[u32]]) -> Simulator {
    simulator_with_programs_on(backend, programs, programs.len())
}

/// A simulator with `hart_count` harts running `programs`, one per hart
/// from hart 0.
fn simulator_with_programs_on(
    backend: BackendKind,
    programs: &[&[u32]],
    hart_count: usize,
) -> Simulator {
    let mut config = config(backend);
    config.system.hart_count = hart_count;
    let mut sim = Simulator::build(&config, "");
    let args = ProcessArgs { argv: vec!["prog".to_string()], envp: vec![] };
    let images: Vec<Vec<u8>> = programs.iter().map(|code| user_elf(code, ET_EXEC)).collect();
    let loaded: Vec<ProgramImage<'_>> =
        images.iter().map(|data| ProgramImage { data, args: &args }).collect();
    sim.load_processes(&loaded).expect("the programs load");
    sim
}

/// Counts to `count` in a loop, then exits with `code`.
fn count_then_exit(count: i32, code: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 0).build(),
        i().addi(T1, 0, count).build(),
        i().addi(T0, T0, 1).build(),
        i().blt(T0, T1, -4).build(),
        i().addi(A0, 0, code).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ]
}

/// Stores `value` at a fixed address in its own data page, reads it back
/// after a pause, and exits with it: the pages of two programs at the same
/// virtual address are different frames.
fn store_then_exit_with_load(value: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, value).build(),
        i().sd(SP, T0, -32).build(),
        i().addi(T1, 0, 200).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -4).build(),
        i().ld(A0, SP, -32).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ]
}

/// Two programs run on two harts; the run ends when the second one exits,
/// with the first non-zero exit code in hart order, and each hart reports
/// its own program's code.
#[test]
fn two_programs_run_to_their_own_exits() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_with_programs(backend, &[&exit_with(0), &count_then_exit(300, 9)]);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(9), "{backend:?}");
        assert_eq!(sim.exit_codes(), vec![Some(0), Some(9)], "{backend:?}");
        let retired = |hart: usize| sim.hart(hart).instructions_retired;
        assert!(
            retired(1) > retired(0) + 500,
            "{backend:?}: hart 1 kept running after hart 0 exited"
        );
    }
}

#[test]
fn the_runs_exit_code_is_the_first_non_zero_one_in_program_order() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim =
            simulator_with_programs(backend, &[&exit_with(5), &exit_with(7), &exit_with(0)]);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(5), "{backend:?}");
        assert_eq!(sim.exit_codes(), vec![Some(5), Some(7), Some(0)], "{backend:?}");
    }
}

#[test]
fn each_program_has_its_own_memory() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_with_programs(
            backend,
            &[&store_then_exit_with_load(11), &store_then_exit_with_load(22)],
        );

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(11), "{backend:?}");
        assert_eq!(sim.exit_codes(), vec![Some(11), Some(22)], "{backend:?}");
    }
}

/// A faulting program ends with its signal's status while the other goes
/// on to its own exit.
#[test]
fn a_fault_ends_one_program_and_the_other_runs_on() {
    let illegal = vec![0xffff_ffff];
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_with_programs(backend, &[&illegal, &count_then_exit(300, 0)]);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(132), "{backend:?}");
        assert_eq!(sim.exit_codes(), vec![Some(132), Some(0)], "{backend:?}");
    }
}

/// `exit_codes` names programs, not harts: a hart without a program has
/// no entry.
#[test]
fn exit_codes_hold_one_code_per_program() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_with_programs_on(backend, &[&exit_with(4), &exit_with(6)], 3);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(4), "{backend:?}");
        assert_eq!(sim.exit_codes(), vec![Some(4), Some(6)], "{backend:?}");
    }
}

/// Exits with `16 * getpid() + gettid()`.
fn exit_with_ids() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(A7, 0, SYS_GETPID).build(),
        ECALL,
        i().addi(T1, 0, 16).build(),
        i().mul(T0, A0, T1).build(),
        i().addi(A7, 0, SYS_GETTID).build(),
        ECALL,
        i().add(A0, A0, T0).build(),
        i().addi(A7, 0, SYS_EXIT).build(),
        ECALL,
    ]
}

/// Each program is a process whose first thread's id is the process's:
/// 1 for the first program, 2 for the second.
#[test]
fn a_programs_first_thread_has_the_programs_id() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut sim = simulator_with_programs(backend, &[&exit_with_ids(), &exit_with_ids()]);

        let _ = run_for(&mut sim, 100_000);

        assert_eq!(sim.exit_codes(), vec![Some(16 + 1), Some(2 * 16 + 2)], "{backend:?}");
    }
}

#[test]
fn more_programs_than_harts_are_refused() {
    let config = config(BackendKind::InOrder);
    let mut sim = Simulator::build(&config, "");
    let args = ProcessArgs::default();
    let a = user_elf(&exit_with(0), ET_EXEC);
    let programs = [ProgramImage { data: &a, args: &args }, ProgramImage { data: &a, args: &args }];

    let error = sim.load_processes(&programs).expect_err("two programs on one hart");

    assert!(matches!(error, SimError::UnsupportedProgram { .. }), "{error}");
}

/// A checkpoint taken mid-run of two programs resumes both to the same
/// end as the uninterrupted run.
#[test]
fn a_checkpoint_resumes_two_programs() {
    let programs: [&[u32]; 2] = [&count_then_exit(400, 3), &count_then_exit(900, 0)];
    let mut whole = simulator_with_programs(BackendKind::OutOfOrder, &programs);
    let expected = run_for(&mut whole, 100_000);
    let mut first = simulator_with_programs(BackendKind::OutOfOrder, &programs);
    let _ = run_for(&mut first, 200);
    let mut saved = Vec::new();
    first.save_checkpoint(&mut saved).expect("the checkpoint saves");
    let mut second = simulator_with_programs(BackendKind::OutOfOrder, &programs);
    second.restore_checkpoint(&mut std::io::Cursor::new(saved)).expect("the checkpoint restores");

    let resumed = run_for(&mut second, 100_000);

    assert_eq!(expected, StopReason::Exited(3));
    assert_eq!(resumed, expected);
    assert_eq!(second.exit_codes(), whole.exit_codes());
    assert_eq!(second.state.instructions_retired(), whole.state.instructions_retired());
}
