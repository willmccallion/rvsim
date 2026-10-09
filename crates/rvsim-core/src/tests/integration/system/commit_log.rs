//! The commit log a lockstep checker replays carries each retired
//! instruction's register, CSR and memory effects and each trap, the same
//! on every backend.

use std::collections::HashMap;

use crate::config::{BackendKind, Config};
use crate::tests::support::builder::instruction::{ECALL, InstructionBuilder};
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const DATA: u64 = PROGRAM_BASE + 0x1000;
const HANDLER: u64 = PROGRAM_BASE + 0x50;
const VALUE: u64 = 0x7f;
const T0: u32 = 5;
const T1: u32 = 6;
const MSCRATCH: u32 = 0x340;
const MSTATUS: u32 = 0x300;
const MTVEC: u32 = 0x305;
const ECALL_FROM_M: u64 = 11;

const FFLAGS_DZ: u64 = 0x08;

/// `fmv.d.x f1, t1`.
const FMV_D_X_F1_T1: u32 = 0xf200_0053 | (T1 << 15) | (1 << 7);
/// `fdiv.d f2, f1, f0` rounding to nearest: a division by zero.
const FDIV_D_F2_F1_F0: u32 = 0x1a00_0053 | (1 << 15) | (2 << 7);
/// `vsetvli a5, zero, e64, m1`.
const VSETVLI_A5_E64: u32 = 0x57 | (15 << 7) | (0b111 << 12) | (0x18 << 20);

/// Stores, loads, an AMO, an LR/SC pair, CSR writes, FP and vector
/// instructions, then an ECALL into a handler at `HANDLER` that spins.
fn every_effect() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T0, 1).build(),
        i().addi(T1, 0, VALUE as i32).build(),
        i().sd(T0, T1, 0).build(),
        i().ld(7, T0, 0).build(),
        i().sb(T0, T1, 9).build(),
        i().lbu(8, T0, 9).build(),
        i().amoadd_d(9, T0, T1).build(),
        i().lr_d(10, T0).build(),
        i().sc_d(11, T0, T1).build(),
        i().csrrw(12, MSCRATCH, T1).build(),
        i().lui(13, 0x6).build(),
        i().addi(13, 13, 0x600).build(),
        i().csrrs(0, MSTATUS, 13).build(),
        FMV_D_X_F1_T1,
        FDIV_D_F2_F1_F0,
        VSETVLI_A5_E64,
        i().auipc(14, 0).build(),
        i().addi(14, 14, 16).build(),
        i().csrrw(0, MTVEC, 14).build(),
        ECALL,
        i().jal(0, 0).build(),
    ]
}

/// Runs `every_effect` on `backend` with the commit log open and returns
/// the log's lines.
fn logged_lines(backend: BackendKind, width: usize) -> Vec<String> {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    let log = tempfile::NamedTempFile::new().expect("temp file");
    let path = log.path().to_str().expect("utf-8 path").to_owned();
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &every_effect());
    ctx.sim.open_commit_log(&path).expect("open commit log");

    let reached = ctx.run_until(5_000, |ctx| ctx.cpu().harts[0].pc == HANDLER);
    ctx.run(20);
    drop(ctx);

    assert!(reached.is_some(), "{backend:?} width {width}: the ECALL never reached its handler");
    std::fs::read_to_string(&path).expect("read commit log").lines().map(str::to_owned).collect()
}

/// The effects logged after each instruction's `priv` field, by PC.
fn effects_by_pc(lines: &[String]) -> HashMap<u64, String> {
    lines
        .iter()
        .filter_map(|line| {
            let rest = line.strip_prefix("core   0: 0x")?;
            let pc = u64::from_str_radix(&rest[..16], 16).ok()?;
            let effects = rest.split_once(" priv 3")?.1.trim().to_owned();
            Some((pc, effects))
        })
        .collect()
}

fn mem(kind: &str, addr: u64, bytes: u64, value: u64) -> String {
    format!("{kind} 0x{addr:016x} 0x{addr:016x} {bytes} 0x{value:016x}")
}

#[test]
fn each_memory_access_is_logged_with_its_address_bytes_and_value() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));
    let at = |index: u64| effects[&(PROGRAM_BASE + 4 * index)].clone();

    assert_eq!(at(2), mem("store", DATA, 8, VALUE));
    assert_eq!(at(3), format!("x7 0x{VALUE:016x} {}", mem("load", DATA, 8, VALUE)));
    assert_eq!(at(4), mem("store", DATA + 9, 1, VALUE));
    assert_eq!(at(5), format!("x8 0x{VALUE:016x} {}", mem("load", DATA + 9, 1, VALUE)));
    assert_eq!(
        at(6),
        format!(
            "x9 0x{VALUE:016x} {} {}",
            mem("load", DATA, 8, VALUE),
            mem("store", DATA, 8, 2 * VALUE)
        )
    );
    assert_eq!(at(7), format!("x10 0x{:016x} {}", 2 * VALUE, mem("load", DATA, 8, 2 * VALUE)));
    assert_eq!(at(8), format!("x11 0x{:016x} {}", 0, mem("store", DATA, 8, VALUE)));
}

#[test]
fn a_csr_write_logs_the_value_read_back_after_it() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));

    assert_eq!(effects[&(PROGRAM_BASE + 4 * 9)], format!("x12 0x{:016x} c340 0x{VALUE:016x}", 0));
    assert!(
        effects[&(PROGRAM_BASE + 4 * 12)].starts_with("c300 0x"),
        "the mstatus write logged {:?}",
        effects[&(PROGRAM_BASE + 4 * 12)]
    );
}

#[test]
fn an_fp_destination_is_logged_as_an_f_register() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));

    assert_eq!(effects[&(PROGRAM_BASE + 4 * 13)], format!("f1 0x{VALUE:016x}"));
}

#[test]
fn raised_fp_flags_are_logged_as_the_fflags_they_leave() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));

    assert_eq!(
        effects[&(PROGRAM_BASE + 4 * 14)],
        format!("f2 0x{:016x} c001 0x{FFLAGS_DZ:016x}", f64::INFINITY.to_bits())
    );
}

#[test]
fn a_vector_instruction_is_marked_and_keeps_its_scalar_result() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));
    let vsetvli = &effects[&(PROGRAM_BASE + 4 * 15)];

    assert!(
        vsetvli.starts_with("x15 0x") && vsetvli.ends_with(" vec"),
        "vsetvli logged {vsetvli:?}"
    );
}

#[test]
fn the_log_opens_with_the_state_it_starts_from() {
    let lines = logged_lines(BackendKind::InOrder, 1);
    let reset_csrs: Vec<String> = lines
        .iter()
        .filter_map(|line| line.strip_prefix("core   0: reset c"))
        .map(|rest| rest[..3].to_owned())
        .collect();
    let expected: Vec<String> = crate::uarch::pipeline::commit_log::RESET_CSRS
        .iter()
        .map(|addr| format!("{:03x}", addr.as_u32()))
        .collect();

    assert_eq!(lines[0], format!("core   0: reset pc 0x{PROGRAM_BASE:016x} priv 3"));
    assert_eq!(reset_csrs, expected);
}

#[test]
fn a_trap_is_logged_after_the_instruction_that_raised_it() {
    let lines = logged_lines(BackendKind::InOrder, 1);
    let ecall_pc = PROGRAM_BASE + 4 * 19;
    let ecall = lines
        .iter()
        .position(|line| line.starts_with(&format!("core   0: 0x{ecall_pc:016x} ")))
        .expect("the ECALL is logged");

    assert_eq!(lines[ecall], format!("core   0: 0x{ecall_pc:016x} (0x{ECALL:08x}) priv 3"));
    assert_eq!(
        lines[ecall + 1],
        format!("core   0: trap 0x{ECALL_FROM_M:016x} 0x{ecall_pc:016x} 0x{:016x}", 0)
    );
}

/// The lines up to the handler's first instruction; how often the handler
/// spins after it depends on timing.
fn up_to_handler(lines: &[String]) -> Vec<String> {
    let handler = format!("core   0: 0x{HANDLER:016x} ");
    let end = lines.iter().position(|line| line.starts_with(&handler)).expect("the handler ran");
    lines[..=end].to_vec()
}

#[test]
fn every_backend_logs_the_same_retirements() {
    let reference = up_to_handler(&logged_lines(BackendKind::InOrder, 1));

    for (backend, width) in
        [(BackendKind::InOrder, 4), (BackendKind::OutOfOrder, 1), (BackendKind::OutOfOrder, 4)]
    {
        let lines = up_to_handler(&logged_lines(backend, width));

        assert_eq!(lines, reference, "{backend:?} width {width}");
    }
}
