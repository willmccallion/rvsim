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

/// `madvise` takes the advice Linux 6.6 knows and refuses the rest, as
/// that kernel refuses glibc 2.42's guard-page request (102), so glibc
/// falls back to `mprotect` for a thread's stack guard as it does there.
#[test]
fn madvise_refuses_advice_linux_6_6_does_not_know() {
    const SYS_MADVISE: i32 = 233;
    const MADV_DONTNEED: i32 = 4;
    const MADV_GUARD_INSTALL: i32 = 102;
    let i = InstructionBuilder::new;
    let advise = |advice: i32| {
        [
            i().lui(A0, (LOAD_ADDRESS >> 12) as i32).build(),
            i().lui(A1, 1).build(),
            i().addi(A2, 0, advice).build(),
            i().addi(A7, 0, SYS_MADVISE).build(),
            ECALL,
        ]
    };
    let mut code = advise(MADV_GUARD_INSTALL).to_vec();
    code.push(i().sub(T1, 0, A0).build());
    code.extend(advise(MADV_DONTNEED));
    code.extend([i().add(A0, A0, T1).build(), i().addi(A7, 0, SYS_EXIT).build(), ECALL]);
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &code, &["prog"]);

        assert_eq!(stop, StopReason::Exited(22), "{backend:?}: EINVAL, then 0");
    }
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
    simulator_with_programs_in(config(backend), programs, hart_count)
}

/// A simulator on `config` with `hart_count` harts running `programs`, one
/// per hart from hart 0.
fn simulator_with_programs_in(
    mut config: Config,
    programs: &[&[u32]],
    hart_count: usize,
) -> Simulator {
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

const S0: u32 = 8;
const S1: u32 = 9;
const S2: u32 = 18;
const SYS_CLONE: i32 = 220;
const SYS_FUTEX: i32 = 98;
const SYS_EXIT_GROUP: i32 = 94;
const SYS_SCHED_GETAFFINITY: i32 = 123;
const SYS_TGKILL: i32 = 131;
const SYS_MMAP: i32 = 222;
/// `CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD`.
const CLONE_THREAD_FLAGS: i32 = 0x1_0F00;
/// A thread's flags with `CLONE_PARENT_SETTID | CLONE_CHILD_CLEARTID`, as
/// `pthread_create` passes them for `pthread_join` to wait on.
const CLONE_JOINABLE_FLAGS: i32 = CLONE_THREAD_FLAGS | 0x30_0000;
const FUTEX_WAIT: i32 = 0;
const ETIMEDOUT: u64 = 110;
const EAGAIN: u64 = 11;

/// A branch or jump to a label of a test program.
#[derive(Clone, Copy)]
enum Jump {
    Beq(u32, u32),
    Bne(u32, u32),
    Blt(u32, u32),
    Jal,
}

/// A line of a test program: an instruction, a label, a jump to a label,
/// or `rd` set to a label's address.
#[derive(Clone, Copy)]
enum Line {
    Inst(u32),
    Label(&'static str),
    To(Jump, &'static str),
    AddressOf(u32, &'static str),
}

/// The instructions of `lines`, with each jump's and address's offset to
/// its label resolved.
fn assemble(lines: &[Line]) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut labels = std::collections::HashMap::new();
    let mut index = 0;
    for line in lines {
        match line {
            Line::Label(name) => assert!(labels.insert(*name, index).is_none(), "{name} twice"),
            Line::AddressOf(..) => index += 2,
            Line::Inst(_) | Line::To(..) => index += 1,
        }
    }
    let mut code = Vec::new();
    for line in lines {
        let offset_to = |name: &str, at: usize| (labels[name] - at as i32) * 4;
        match *line {
            Line::Inst(word) => code.push(word),
            Line::Label(_) => {}
            Line::To(jump, name) => {
                let offset = offset_to(name, code.len());
                code.push(match jump {
                    Jump::Beq(a, b) => i().beq(a, b, offset).build(),
                    Jump::Bne(a, b) => i().bne(a, b, offset).build(),
                    Jump::Blt(a, b) => i().blt(a, b, offset).build(),
                    Jump::Jal => i().jal(0, offset).build(),
                });
            }
            Line::AddressOf(rd, name) => {
                let offset = offset_to(name, code.len());
                code.push(i().auipc(rd, 0).build());
                code.push(i().addi(rd, rd, offset).build());
            }
        }
    }
    code
}

fn inst(word: u32) -> Line {
    Line::Inst(word)
}

/// `rd = value`, for a value `lui` and `addi` can build.
fn load_immediate(rd: u32, value: i32) -> [Line; 2] {
    let i = InstructionBuilder::new;
    let upper = (value + 0x800) >> 12;
    [inst(i().lui(rd, upper).build()), inst(i().addi(rd, rd, value - (upper << 12)).build())]
}

/// `syscall(number)` with the arguments already in place.
fn syscall(number: i32) -> [Line; 2] {
    [inst(InstructionBuilder::new().addi(A7, 0, number).build()), inst(ECALL)]
}

/// `clone(flags, sp - 64 KiB, s0, 0, s0)`: a thread on a stack below the
/// caller's, with `s0` as its parent and child id word.
fn clone_thread(flags: i32) -> Vec<Line> {
    let i = InstructionBuilder::new;
    let mut lines = load_immediate(A0, flags).to_vec();
    lines.extend(load_immediate(T0, 0x1_0000));
    lines.extend([
        inst(i().sub(A1, SP, T0).build()),
        inst(i().addi(A2, S0, 0).build()),
        inst(i().addi(A3, 0, 0).build()),
        inst(i().addi(A4, S0, 0).build()),
    ]);
    lines.extend(syscall(SYS_CLONE));
    lines
}

/// Waits, as `pthread_join` does, until the word at `s0` is zero, on a
/// futex while it holds the thread's id.
fn join(wait: &'static str, joined: &'static str) -> Vec<Line> {
    let i = InstructionBuilder::new;
    let mut lines = vec![
        Line::Label(wait),
        inst(i().lw(A2, S0, 0).build()),
        Line::To(Jump::Beq(A2, 0), joined),
        inst(i().addi(A0, S0, 0).build()),
        inst(i().addi(A1, 0, FUTEX_WAIT).build()),
        inst(i().addi(A3, 0, 0).build()),
    ];
    lines.extend(syscall(SYS_FUTEX));
    lines.extend([Line::To(Jump::Jal, wait), Line::Label(joined)]);
    lines
}

/// Counts `t0` up to `count`.
fn spin(count: i32, label: &'static str) -> Vec<Line> {
    let i = InstructionBuilder::new;
    let mut lines = vec![inst(i().addi(T0, 0, 0).build())];
    lines.extend(load_immediate(T1, count));
    lines.extend([
        Line::Label(label),
        inst(i().addi(T0, T0, 1).build()),
        Line::To(Jump::Blt(T0, T1), label),
    ]);
    lines
}

/// `exit(code)` or `exit_group(code)` with `code` an immediate.
fn exit_lines(number: i32, code: i32) -> Vec<Line> {
    let mut lines = vec![inst(InstructionBuilder::new().addi(A0, 0, code).build())];
    lines.extend(syscall(number));
    lines
}

/// A machine the thread tests run on: a backend, with or without private
/// L1 caches and an L2 per core, which the coherence fabric keeps coherent
/// between harts.
#[derive(Clone, Copy, Debug)]
struct Machine {
    backend: BackendKind,
    caches: bool,
}

const MACHINES: [Machine; 4] = [
    Machine { backend: BackendKind::InOrder, caches: false },
    Machine { backend: BackendKind::InOrder, caches: true },
    Machine { backend: BackendKind::OutOfOrder, caches: false },
    Machine { backend: BackendKind::OutOfOrder, caches: true },
];

impl Machine {
    fn config(self) -> Config {
        let mut config = config(self.backend);
        config.cache.l1_i.enabled = self.caches;
        config.cache.l1_d.enabled = self.caches;
        config.cache.l2.enabled = self.caches;
        config
    }
}

/// A simulator with `hart_count` harts of `machine` running `code` as one
/// program.
fn threaded(machine: Machine, code: &[u32], hart_count: usize) -> Simulator {
    simulator_with_programs_in(machine.config(), &[code], hart_count)
}

/// A cloned thread starts on the free hart and sees its parent's memory:
/// it stores 42 where the parent waits for it.
#[test]
fn a_cloned_thread_runs_on_a_free_hart_and_shares_its_memory() {
    let i = InstructionBuilder::new;
    let mut lines = vec![Line::AddressOf(S0, "word")];
    lines.extend(clone_thread(CLONE_THREAD_FLAGS));
    lines.extend([
        Line::To(Jump::Beq(A0, 0), "child"),
        Line::Label("wait"),
        inst(i().lw(A0, S0, 0).build()),
        Line::To(Jump::Beq(A0, 0), "wait"),
    ]);
    lines.extend(syscall(SYS_EXIT_GROUP));
    lines.extend([
        Line::Label("child"),
        inst(i().addi(T1, 0, 42).build()),
        inst(i().sw(S0, T1, 0).build()),
    ]);
    lines.extend(exit_lines(SYS_EXIT, 0));
    lines.extend([Line::Label("word"), inst(0)]);
    let code = assemble(&lines);
    for machine in MACHINES {
        let mut sim = threaded(machine, &code, 2);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(42), "{machine:?}");
        assert_eq!(sim.exit_codes(), vec![Some(42)], "{machine:?}");
        assert!(sim.hart(1).instructions_retired > 0, "{machine:?}: the thread ran on hart 1");
    }
}

/// The `pthread_join` protocol: `CLONE_PARENT_SETTID` stores the child's
/// id in the word, the joiner waits on the word, and the child's exit
/// zeroes it and wakes the joiner. The run exits with the id twice. The
/// child spins long enough, on the uncached test configuration, for the
/// joiner to be waiting by the time it exits.
fn join_a_slow_thread() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut lines = vec![Line::AddressOf(S0, "word")];
    lines.extend(clone_thread(CLONE_JOINABLE_FLAGS));
    lines.extend([
        Line::To(Jump::Beq(A0, 0), "child"),
        inst(i().addi(S1, A0, 0).build()),
        inst(i().lw(S2, S0, 0).build()),
    ]);
    lines.extend(join("join", "joined"));
    lines.push(inst(i().add(A0, S1, S2).build()));
    lines.extend(syscall(SYS_EXIT_GROUP));
    lines.push(Line::Label("child"));
    lines.extend(spin(300, "spin"));
    lines.extend(exit_lines(SYS_EXIT, 0));
    lines.extend([Line::Label("word"), inst(0)]);
    assemble(&lines)
}

#[test]
fn a_joiner_waits_on_the_thread_id_word_until_the_thread_exits() {
    let code = join_a_slow_thread();
    for machine in MACHINES {
        let mut sim = threaded(machine, &code, 2);

        let stop = run_for(&mut sim, 400_000);

        assert_eq!(stop, StopReason::Exited(2 + 2), "{machine:?}");
    }
}

/// SE mode runs one thread per hart, as gem5 does: with every hart taken,
/// `clone` fails with `EAGAIN`.
#[test]
fn clone_fails_with_eagain_when_every_hart_is_taken() {
    let i = InstructionBuilder::new;
    let mut lines = vec![Line::AddressOf(S0, "word")];
    lines.extend(clone_thread(CLONE_THREAD_FLAGS));
    lines.push(inst(i().sub(A0, 0, A0).build()));
    lines.extend(syscall(SYS_EXIT));
    lines.extend([Line::Label("word"), inst(0)]);
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 1), 100_000);

        assert_eq!(stop, StopReason::Exited(EAGAIN), "{machine:?}");
    }
}

/// A thread's `gettid` is its own id and its `getpid` its process's; its
/// `exit_group` ends the process though the first thread still runs.
#[test]
fn a_thread_has_its_own_id_and_its_process_id() {
    let i = InstructionBuilder::new;
    let mut lines = clone_thread(CLONE_THREAD_FLAGS);
    lines.extend([
        Line::To(Jump::Beq(A0, 0), "child"),
        Line::Label("forever"),
        Line::To(Jump::Jal, "forever"),
        Line::Label("child"),
    ]);
    lines.extend(syscall(SYS_GETTID));
    lines.push(inst(i().addi(S1, A0, 0).build()));
    lines.extend(syscall(SYS_GETPID));
    lines.extend([
        inst(i().addi(T1, 0, 16).build()),
        inst(i().mul(S1, S1, T1).build()),
        inst(i().add(A0, A0, S1).build()),
    ]);
    lines.extend(syscall(SYS_EXIT_GROUP));
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 2), 100_000);

        assert_eq!(stop, StopReason::Exited(2 * 16 + 1), "{machine:?}");
    }
}

/// `exit` ends one thread: the process lives on in its other thread, and
/// when that one exits too the process ends with its first thread's code,
/// as Linux reports it.
#[test]
fn exit_ends_one_thread_and_the_last_ends_the_process_with_the_first_ones_code() {
    let mut lines = clone_thread(CLONE_THREAD_FLAGS);
    lines.push(Line::To(Jump::Beq(A0, 0), "child"));
    lines.extend(exit_lines(SYS_EXIT, 3));
    lines.push(Line::Label("child"));
    lines.extend(spin(2000, "spin"));
    lines.extend(exit_lines(SYS_EXIT, 7));
    let code = assemble(&lines);
    for machine in MACHINES {
        let mut sim = threaded(machine, &code, 2);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(3), "{machine:?}");
        assert!(sim.hart(1).instructions_retired > 2000, "{machine:?}: the thread ran on");
    }
}

/// `exit_group` stops the other threads of its process at once and leaves
/// other programs running.
#[test]
fn exit_group_stops_its_processes_other_threads_only() {
    let mut lines = clone_thread(CLONE_THREAD_FLAGS);
    lines.extend([Line::To(Jump::Beq(A0, 0), "child")]);
    lines.extend(spin(200, "settle"));
    lines.extend(exit_lines(SYS_EXIT_GROUP, 5));
    lines.extend([Line::Label("child"), Line::Label("forever"), Line::To(Jump::Jal, "forever")]);
    let threaded_program = assemble(&lines);
    let counter = count_then_exit(2000, 0);
    for machine in MACHINES {
        let mut sim =
            simulator_with_programs_in(machine.config(), &[&threaded_program, &counter], 3);

        let stop = run_for(&mut sim, 200_000);

        assert_eq!(stop, StopReason::Exited(5), "{machine:?}");
        assert_eq!(sim.exit_codes(), vec![Some(5), Some(0)], "{machine:?}");
        assert!(sim.hart(2).instructions_retired > 0, "{machine:?}: the thread took hart 2");
        assert!(sim.hart(2).wfi_waiting, "{machine:?}: the thread's hart parked");
        assert!(sim.hart(1).instructions_retired > 2000, "{machine:?}: the other program ran on");
    }
}

/// A fault in any thread ends the whole process, as its signal would.
#[test]
fn a_fault_in_a_thread_ends_its_whole_process() {
    let mut lines = clone_thread(CLONE_THREAD_FLAGS);
    lines.extend([
        Line::To(Jump::Beq(A0, 0), "child"),
        Line::Label("forever"),
        Line::To(Jump::Jal, "forever"),
        Line::Label("child"),
        inst(0xffff_ffff),
    ]);
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 2), 100_000);

        assert_eq!(stop, StopReason::Exited(128 + 4), "{machine:?}");
    }
}

/// A futex wait on the stack word with the `timespec` `{0, nanos}` beside
/// it, its result negated as the exit code.
fn wait_on_a_stack_word(word: i32, nanos: i32) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut lines = vec![
        inst(i().sd(SP, 0, -16).build()),
        inst(i().addi(T0, 0, nanos).build()),
        inst(i().sd(SP, T0, -8).build()),
        inst(i().addi(T0, 0, word).build()),
        inst(i().sw(SP, T0, -24).build()),
        inst(i().addi(A0, SP, -24).build()),
        inst(i().addi(A1, 0, FUTEX_WAIT).build()),
        inst(i().addi(A2, 0, 0).build()),
        inst(i().addi(A3, SP, -16).build()),
    ];
    lines.extend(syscall(SYS_FUTEX));
    lines.push(inst(i().sub(A0, 0, A0).build()));
    lines.extend(syscall(SYS_EXIT));
    assemble(&lines)
}

/// A wait nobody wakes ends with `ETIMEDOUT` once its timeout has passed
/// in simulated time: 2 µs, 2000 cycles at 1 GHz.
#[test]
fn a_futex_wait_times_out_after_its_timeout() {
    let code = wait_on_a_stack_word(0, 2000);
    for machine in MACHINES {
        let mut sim = threaded(machine, &code, 1);

        let stop = run_for(&mut sim, 100_000);

        assert_eq!(stop, StopReason::Exited(ETIMEDOUT), "{machine:?}");
        assert!(sim.cycle() >= 2000, "{machine:?}: the wait lasted {} cycles", sim.cycle());
    }
}

#[test]
fn a_futex_wait_on_a_word_that_changed_returns_eagain() {
    let code = wait_on_a_stack_word(1, 2000);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 1), 100_000);

        assert_eq!(stop, StopReason::Exited(EAGAIN), "{machine:?}");
    }
}

/// A hart is free again once its thread has exited: a second thread
/// started after joining the first takes it, with the next id.
#[test]
fn a_freed_hart_takes_the_next_thread() {
    let i = InstructionBuilder::new;
    let mut lines = vec![Line::AddressOf(S0, "word")];
    lines.extend(clone_thread(CLONE_JOINABLE_FLAGS));
    lines.push(Line::To(Jump::Beq(A0, 0), "child"));
    lines.extend(join("join_first", "joined_first"));
    lines.extend(clone_thread(CLONE_JOINABLE_FLAGS));
    lines.extend([Line::To(Jump::Beq(A0, 0), "child"), inst(i().addi(S1, A0, 0).build())]);
    lines.extend(join("join_second", "joined_second"));
    lines.push(inst(i().addi(A0, S1, 0).build()));
    lines.extend(syscall(SYS_EXIT_GROUP));
    lines.push(Line::Label("child"));
    lines.extend(exit_lines(SYS_EXIT, 0));
    lines.extend([Line::Label("word"), inst(0)]);
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 2), 200_000);

        assert_eq!(stop, StopReason::Exited(3), "{machine:?}");
    }
}

/// `sched_getaffinity` returns the 8 bytes of a 64-CPU kernel's mask with
/// a bit for every hart a thread may start on.
#[test]
fn sched_getaffinity_reports_every_hart() {
    let i = InstructionBuilder::new;
    let mut lines = vec![
        inst(i().addi(A0, 0, 0).build()),
        inst(i().addi(A1, 0, 128).build()),
        inst(i().addi(A2, SP, -128).build()),
    ];
    lines.extend(syscall(SYS_SCHED_GETAFFINITY));
    lines.extend([
        inst(i().lbu(T0, SP, -128).build()),
        inst(i().addi(T1, 0, 16).build()),
        inst(i().mul(A0, A0, T1).build()),
        inst(i().add(A0, A0, T0).build()),
    ]);
    lines.extend(syscall(SYS_EXIT));
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 3), 100_000);

        assert_eq!(stop, StopReason::Exited(8 * 16 + 0b111), "{machine:?}");
    }
}

/// `tgkill(tgid, tid, signal)` from the first thread to `tid`.
fn tgkill_self(tid: i32, signal: i32) -> [Line; 5] {
    let i = InstructionBuilder::new;
    let [set, call] = syscall(SYS_TGKILL);
    [
        inst(i().addi(A0, 0, 1).build()),
        inst(i().addi(A1, 0, tid).build()),
        inst(i().addi(A2, 0, signal).build()),
        set,
        call,
    ]
}

/// Signal 0 checks that the thread exists and a signal ignored by default
/// does nothing; the exit code is what the missing thread's probe returned.
#[test]
fn tgkill_with_signal_zero_checks_that_the_thread_exists() {
    let i = InstructionBuilder::new;
    let mut lines = tgkill_self(1, 0).to_vec();
    lines.push(inst(i().addi(S1, A0, 0).build()));
    lines.extend(tgkill_self(1, 17));
    lines.push(inst(i().add(S1, S1, A0).build()));
    lines.extend(tgkill_self(99, 0));
    lines.push(inst(i().sub(A0, S1, A0).build()));
    lines.extend(syscall(SYS_EXIT));
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 1), 100_000);

        assert_eq!(stop, StopReason::Exited(3), "{machine:?}: ESRCH, and nothing else");
    }
}

#[test]
fn a_fatal_signal_to_a_thread_ends_its_process() {
    let mut lines = tgkill_self(1, 6).to_vec();
    lines.extend(exit_lines(SYS_EXIT, 0));
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 1), 100_000);

        assert_eq!(stop, StopReason::Exited(128 + 6), "{machine:?}");
    }
}

/// `mmap(addr, 4096, PROT_READ | PROT_WRITE, flags, -1, 0)` with `addr`
/// in `a0`.
fn map_a_page(flags: i32) -> Vec<Line> {
    let i = InstructionBuilder::new;
    let mut lines = vec![inst(i().lui(A1, 1).build())];
    lines.extend([
        inst(i().addi(A2, 0, 3).build()),
        inst(i().addi(A3, 0, flags).build()),
        inst(i().addi(A4, 0, -1).build()),
        inst(i().addi(A5, 0, 0).build()),
    ]);
    lines.extend(syscall(SYS_MMAP));
    lines
}

/// When one thread replaces a page another has been reading, the reader's
/// hart drops its translations as a kernel's shootdown would make it, and
/// reads the new page's zeros; with its stale TLB entry it would read the
/// old page's 5 for ever.
#[test]
fn replacing_a_mapping_drops_the_other_threads_translations() {
    const MAP_PRIVATE_ANONYMOUS: i32 = 0x22;
    const MAP_FIXED: i32 = 0x10;
    let i = InstructionBuilder::new;
    let mut lines = vec![inst(i().addi(A0, 0, 0).build())];
    lines.extend(map_a_page(MAP_PRIVATE_ANONYMOUS));
    lines.extend([
        inst(i().addi(S0, A0, 0).build()),
        inst(i().addi(T0, 0, 5).build()),
        inst(i().sw(S0, T0, 0).build()),
    ]);
    lines.extend(clone_thread(CLONE_THREAD_FLAGS));
    lines.push(Line::To(Jump::Beq(A0, 0), "child"));
    lines.extend(spin(500, "settle"));
    lines.push(inst(i().addi(A0, S0, 0).build()));
    lines.extend(map_a_page(MAP_PRIVATE_ANONYMOUS | MAP_FIXED));
    lines.extend([
        Line::Label("forever"),
        Line::To(Jump::Jal, "forever"),
        Line::Label("child"),
        Line::Label("poll"),
        inst(i().lw(T0, S0, 0).build()),
        Line::To(Jump::Bne(T0, 0), "poll"),
    ]);
    lines.extend(exit_lines(SYS_EXIT_GROUP, 1));
    let code = assemble(&lines);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 2), 200_000);

        assert_eq!(stop, StopReason::Exited(1), "{machine:?}");
    }
}

/// A checkpoint taken while the first thread waits for the second
/// restores both, the wait included: the run ends as the uninterrupted
/// one does, with the same instructions retired.
#[test]
fn a_checkpoint_resumes_a_thread_waiting_on_a_futex() {
    let code = join_a_slow_thread();
    for machine in MACHINES {
        let mut whole = threaded(machine, &code, 2);
        let expected = run_for(&mut whole, 400_000);
        let mut first = threaded(machine, &code, 2);
        while !first.hart(0).wfi_waiting {
            assert_eq!(
                run_for(&mut first, 10),
                StopReason::Cycles,
                "{machine:?}: the joiner waits"
            );
        }
        assert!(first.exit_codes()[0].is_none(), "{machine:?}: the thread still runs");
        let mut saved = Vec::new();
        first.save_checkpoint(&mut saved).expect("the checkpoint saves");
        let mut second = threaded(machine, &code, 2);
        second.restore_checkpoint(&mut saved.as_slice()).expect("the checkpoint restores");

        let resumed = run_for(&mut second, 400_000);

        assert_eq!(expected, StopReason::Exited(4), "{machine:?}");
        assert_eq!(resumed, expected, "{machine:?}");
        assert_eq!(second.instructions_retired(), whole.instructions_retired(), "{machine:?}");
    }
}

/// Opens `path` with `flags` and exits with 16 times the bytes a read of
/// it returns plus the digit before its last byte, or with the `errno`
/// the open failed with.
fn read_a_cpu_list(path: &str, flags: i32) -> Vec<u32> {
    const SYS_OPENAT: i32 = 56;
    let i = InstructionBuilder::new;
    let mut lines = vec![
        Line::AddressOf(A1, "path"),
        inst(i().addi(A0, 0, AT_FDCWD).build()),
        inst(i().addi(A2, 0, flags).build()),
        inst(i().addi(A3, 0, 0).build()),
    ];
    lines.extend(syscall(SYS_OPENAT));
    lines.extend([
        Line::To(Jump::Blt(A0, 0), "failed"),
        inst(i().addi(A1, SP, -64).build()),
        inst(i().addi(A2, 0, 16).build()),
    ]);
    lines.extend(syscall(SYS_READ));
    lines.extend([
        inst(i().add(T0, A1, A0).build()),
        inst(i().lbu(T1, T0, -2).build()),
        inst(i().addi(T1, T1, -i32::from(b'0')).build()),
        inst(i().addi(A2, 0, 16).build()),
        inst(i().mul(A0, A0, A2).build()),
        inst(i().add(A0, A0, T1).build()),
    ]);
    lines.extend(syscall(SYS_EXIT));
    lines.extend([Line::Label("failed"), inst(i().sub(A0, 0, A0).build())]);
    lines.extend(syscall(SYS_EXIT));
    lines.push(Line::Label("path"));
    let mut bytes = path.as_bytes().to_vec();
    bytes.resize(bytes.len().next_multiple_of(4) + 4, 0);
    lines.extend(
        bytes.chunks(4).map(|word| inst(u32::from_le_bytes([word[0], word[1], word[2], word[3]]))),
    );
    assemble(&lines)
}

/// The CPU lists glibc's `sysconf(_SC_NPROCESSORS_ONLN)` and `get_nprocs`
/// read name the simulated harts, as gem5 serves them, not the host's
/// CPUs: `0-2` for three harts, `0` for one.
#[test]
fn the_cpu_list_files_name_every_hart() {
    let online = read_a_cpu_list("/sys/devices/system/cpu/online", 0);
    let possible = read_a_cpu_list("/sys/devices/system/cpu/possible", 0);
    for machine in MACHINES {
        let three = run_for(&mut threaded(machine, &online, 3), 100_000);
        let one = run_for(&mut threaded(machine, &possible, 1), 100_000);

        assert_eq!(three, StopReason::Exited(4 * 16 + 2), "{machine:?}: \"0-2\\n\"");
        assert_eq!(one, StopReason::Exited(2 * 16), "{machine:?}: \"0\\n\"");
    }
}

#[test]
fn the_cpu_list_files_are_read_only() {
    const O_WRONLY: i32 = 1;
    const EACCES: u64 = 13;
    let code = read_a_cpu_list("/sys/devices/system/cpu/present", O_WRONLY);
    for machine in MACHINES {
        let stop = run_for(&mut threaded(machine, &code, 2), 100_000);

        assert_eq!(stop, StopReason::Exited(EACCES), "{machine:?}");
    }
}
