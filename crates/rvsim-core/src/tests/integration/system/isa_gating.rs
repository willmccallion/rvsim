//! The decoder refuses an instruction of an extension the configured ISA
//! leaves out, as hardware without the extension would: an
//! illegal-instruction trap, on either backend.

use crate::common::PhysAddr;
use crate::config::{BackendKind, Config, Console};
use crate::tests::support::builder::instruction::{InstructionBuilder, M5_FAIL};
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const HANDLER: u64 = 0x8000_0100;
const A1: u32 = 11;
const MCAUSE: u32 = 0x342;
const ILLEGAL_INSTRUCTION: u64 = 2;
const RAN: i32 = 99;
const JAL_SELF: u32 = 0x0000_006f;
/// `fadd.h f0, f0, f0`.
const FADD_H: u32 = 0x0400_0053;
/// `vsetvli x0, x0, e8`.
const VSETVLI: u32 = 0x0000_7057;
/// `clz x0, x0` (Zbb).
const CLZ: u32 = 0x6000_1013;
const BACKENDS: [BackendKind; 2] = [BackendKind::InOrder, BackendKind::OutOfOrder];

/// Runs `instruction` on a hart with `isa` (the default when `None`) and
/// returns the exit code: `RAN` when it executed, `mcause` when it trapped.
fn exit_code_of(backend: BackendKind, isa: Option<&str>, instruction: u32) -> Option<u64> {
    let i = InstructionBuilder::new;
    let program = [instruction, i().addi(A1, 0, RAN).build(), M5_FAIL, JAL_SELF];
    let handler = [i().csrrs(A1, MCAUSE, 0).build(), M5_FAIL, JAL_SELF];
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    config.isa.string = isa.map(str::to_owned);
    let mut ctx = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program);
    for (n, word) in handler.iter().enumerate() {
        ctx.sim.probe_mem_store(PhysAddr::new(HANDLER + 4 * n as u64), u64::from(*word), 4);
    }
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.run(5_000);
    ctx.sim.state.check_exit()
}

#[test]
fn a_multiply_is_illegal_without_m() {
    let mul = InstructionBuilder::new().mul(0, 1, 2).build();
    for backend in BACKENDS {
        assert_eq!(
            exit_code_of(backend, Some("RV64IAC"), mul),
            Some(ILLEGAL_INSTRUCTION),
            "{backend:?}"
        );
        assert_eq!(exit_code_of(backend, None, mul), Some(RAN as u64), "{backend:?}");
    }
}

#[test]
fn a_half_precision_add_is_illegal_without_f() {
    for backend in BACKENDS {
        assert_eq!(
            exit_code_of(backend, Some("RV64IMAC"), FADD_H),
            Some(ILLEGAL_INSTRUCTION),
            "{backend:?}"
        );
        assert_eq!(exit_code_of(backend, None, FADD_H), Some(RAN as u64), "{backend:?}");
    }
}

#[test]
fn a_vector_configuration_is_illegal_without_v() {
    for backend in BACKENDS {
        assert_eq!(
            exit_code_of(backend, Some("RV64IMAFDC"), VSETVLI),
            Some(ILLEGAL_INSTRUCTION),
            "{backend:?}"
        );
        assert_eq!(exit_code_of(backend, None, VSETVLI), Some(RAN as u64), "{backend:?}");
    }
}

/// A multi-letter extension the ISA string leaves out is gone from the
/// decoder too, and naming it brings it back.
#[test]
fn a_count_leading_zeros_is_illegal_without_zbb() {
    for backend in BACKENDS {
        assert_eq!(
            exit_code_of(backend, Some("rv64gc"), CLZ),
            Some(ILLEGAL_INSTRUCTION),
            "{backend:?}"
        );
        assert_eq!(exit_code_of(backend, Some("rv64gc_zbb"), CLZ), Some(RAN as u64), "{backend:?}");
        assert_eq!(exit_code_of(backend, None, CLZ), Some(RAN as u64), "{backend:?}");
    }
}
