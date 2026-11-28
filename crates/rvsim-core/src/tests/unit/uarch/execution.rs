//! The execution loop's diagnostics: hang detection at one PC, the trace
//! of privilege mode switches and the trace of CSR accesses.

use std::io::Write;
use std::sync::{Arc, Mutex};

use crate::common::{Asid, PhysAddr, Ppn, VirtAddr};
use crate::config::Config;
use crate::config::defaults::RAM_BASE;
use crate::exec::execute::{CsrRequest, perform_csr_access};
use crate::isa::csr;
use crate::isa::encoding::privileged::WFI;
use crate::isa::op::CsrOp;
use crate::isa::privileged::PrivilegeMode;
use crate::soc::bus::HartIrqs;
use crate::system::SystemState;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::uarch::mmu::tlb::PageSize;

const HANG_DETECTION_THRESHOLD: u64 = 5000;
const STUCK_PC_OFFSET: u64 = 0x124;
const PTE_VALID_READ: u64 = 0b11;
const RAM_FRAME: u64 = RAM_BASE >> 12;

/// A `fmt` log sink whose contents the test reads back.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("the log buffer is not poisoned").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Everything `action` logs at any level.
fn logged_during(action: impl FnOnce()) -> String {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer({
            let buffer = buffer.clone();
            move || buffer.clone()
        })
        .finish();
    tracing::subscriber::with_default(subscriber, action);
    let bytes = buffer.0.lock().expect("the log buffer is not poisoned").clone();
    String::from_utf8(bytes).expect("the log is text")
}

fn traced_system() -> SystemState {
    system_tracing(true)
}

fn system_tracing(trace_instructions: bool) -> SystemState {
    let mut config = Config::minimal();
    config.general.trace_instructions = trace_instructions;
    SystemState::build(&config)
}

/// Runs `cycles` cycle starts of a hart that stays at `pc` throughout.
fn log_of_hart_stuck_at(sys: &mut SystemState, pc: VirtAddr, cycles: u64) -> String {
    let mut state = sys.core_ctx(0);
    state.hart.pc = pc;
    state.uncore.per_hart_debug[0].last_pc = pc;
    logged_during(|| {
        for _ in 0..cycles {
            state.pre_tick(HartIrqs::default());
        }
    })
}

#[test]
fn a_hart_that_stays_at_one_pc_for_the_threshold_is_reported_as_hung() {
    let mut sys = traced_system();
    let pc = RAM_BASE + STUCK_PC_OFFSET;
    let nop = InstructionBuilder::new().nop().build();
    sys.uncore.load_binary_at(&nop.to_le_bytes(), PhysAddr::new(pc));

    let log = log_of_hart_stuck_at(&mut sys, VirtAddr::new(pc), HANG_DETECTION_THRESHOLD);

    assert!(log.contains("potential-hang"), "{log}");
    assert!(!log.contains("wfi-wait"), "{log}");
}

#[test]
fn a_hart_that_has_not_yet_stayed_for_the_threshold_is_not_reported() {
    let mut sys = traced_system();

    let log = log_of_hart_stuck_at(&mut sys, VirtAddr::new(RAM_BASE), HANG_DETECTION_THRESHOLD - 1);

    assert_eq!(log, "");
}

#[test]
fn a_hart_that_moves_on_restarts_the_count() {
    let mut sys = traced_system();
    let mut state = sys.core_ctx(0);
    let stuck = VirtAddr::new(RAM_BASE);
    state.hart.pc = stuck;
    state.uncore.per_hart_debug[0].last_pc = stuck;

    let log = logged_during(|| {
        for _ in 0..HANG_DETECTION_THRESHOLD - 1 {
            state.pre_tick(HartIrqs::default());
        }
        state.hart.pc = VirtAddr::new(RAM_BASE + 4);
        state.pre_tick(HartIrqs::default());
        state.hart.pc = stuck;
        state.pre_tick(HartIrqs::default());
    });

    assert_eq!(log, "");
    assert_eq!(state.uncore.per_hart_debug[0].same_pc_count, 0);
}

#[test]
fn a_hart_stuck_on_wfi_is_reported_as_waiting() {
    let mut sys = traced_system();
    let pc = RAM_BASE + STUCK_PC_OFFSET;
    sys.uncore.load_binary_at(&WFI.to_le_bytes(), PhysAddr::new(pc));

    let log = log_of_hart_stuck_at(&mut sys, VirtAddr::new(pc), HANG_DETECTION_THRESHOLD);

    assert!(log.contains("wfi-wait"), "{log}");
    assert!(!log.contains("potential-hang"), "{log}");
}

/// A machine-mode fetch is not translated, so a data TLB entry for the
/// stuck PC's page, here pointing at a frame without the WFI, is no guide.
#[test]
fn a_machine_mode_pc_is_read_untranslated_whatever_the_data_tlb_holds() {
    let mut sys = traced_system();
    let pc = RAM_BASE + STUCK_PC_OFFSET;
    sys.uncore.load_binary_at(&WFI.to_le_bytes(), PhysAddr::new(pc));
    sys.core_ctx(0).core.mmu.dtlb.insert(
        VirtAddr::new(pc).page_number(),
        Ppn::new(RAM_FRAME + 1),
        PTE_VALID_READ,
        Asid::new(0),
        PageSize::Kib4,
    );

    let log = log_of_hart_stuck_at(&mut sys, VirtAddr::new(pc), HANG_DETECTION_THRESHOLD);

    assert!(log.contains("wfi-wait"), "{log}");
}

#[test]
fn a_privilege_switch_in_a_cycle_is_traced() {
    let mut sys = traced_system();
    let state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Supervisor;

    let log = logged_during(|| state.post_tick(PrivilegeMode::Machine));

    assert!(log.contains("mode-switch"), "{log}");
}

#[test]
fn a_cycle_that_keeps_its_privilege_traces_no_switch() {
    let mut sys = traced_system();
    let state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Machine;

    let log = logged_during(|| state.post_tick(PrivilegeMode::Machine));

    assert!(!log.contains("mode-switch"), "{log}");
}

#[test]
fn a_privilege_switch_is_not_traced_when_tracing_is_off() {
    let mut sys = SystemState::build(&Config::minimal());
    let state = sys.core_ctx(0);
    state.hart.privilege = PrivilegeMode::Supervisor;

    let log = logged_during(|| state.post_tick(PrivilegeMode::Machine));

    assert_eq!(log, "");
}

/// What a `csrrw` of `mscratch` logs on a machine that traces
/// instructions or not.
fn csr_write_log(trace_instructions: bool) -> String {
    let mut sys = system_tracing(trace_instructions);
    let mut state = sys.core_ctx(0);
    let request = CsrRequest {
        pc: VirtAddr::new(RAM_BASE),
        addr: csr::MSCRATCH,
        op: CsrOp::Rw,
        src: 1,
        writes: true,
    };

    logged_during(|| {
        let _ = perform_csr_access(&state.stage(), &request);
    })
}

#[test]
fn a_csr_access_is_traced_when_instructions_are() {
    assert!(csr_write_log(true).contains("write-deferred"));
}

#[test]
fn a_csr_access_is_not_traced_when_instructions_are_not() {
    assert_eq!(csr_write_log(false), "");
}
