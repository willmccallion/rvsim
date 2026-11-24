//! Taking a trap costs the configured trap latency, and an interrupt is
//! taken on the instruction at the ROB head, flushing what follows it.

use crate::config::BackendKind;
use crate::config::Config;
use crate::config::Console;
use crate::config::Timer;
use crate::tests::support::builder::instruction::{InstructionBuilder, MRET};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext, store_words};
use crate::tests::support::regs::{A0, A4, T0, T1, T2, T3, T4, T5, T6, X0};

const HANDLER: u64 = PROGRAM_BASE + 0x100;
const MSTATUS: u32 = 0x300;
const MIE: u32 = 0x304;
const TIME: u32 = 0xC01;
const MIE_MTIE: i32 = 1 << 7;
const MSTATUS_MIE: i32 = 1 << 3;
const MACHINE_TIMER_INTERRUPT: u64 = (1 << 63) | 7;
const DIVS: usize = 8;
const ECALL: u32 = 0x0000_0073;

/// Spins.
fn handler() -> Vec<u32> {
    vec![InstructionBuilder::new().jal(X0, 0).build()]
}

/// Arms the machine timer far enough ahead that the refetch after the
/// mstatus write has landed, then runs a chain of divides and a marker,
/// which retire before the timer fires.
fn timer_program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = vec![
        i().lui(T0, 0x02004).build(),
        i().csrrs(T1, TIME, X0).build(),
        i().addi(T1, T1, 16).build(),
        i().sd(T0, T1, 0).build(),
        i().addi(T2, X0, MIE_MTIE).build(),
        i().csrrs(X0, MIE, T2).build(),
        i().addi(T2, X0, MSTATUS_MIE).build(),
        i().csrrs(X0, MSTATUS, T2).build(),
        i().addi(T4, X0, 1).build(),
    ];
    program.extend((0..DIVS).map(|_| i().div(T3, T3, T4).build()));
    program.push(i().addi(T6, X0, 1).build());
    program.push(i().jal(X0, 0).build());
    program
}

fn spin_pc(program: &[u32]) -> u64 {
    PROGRAM_BASE + (program.len() as u64 - 1) * 4
}

struct TrapTaken {
    cycle: u64,
    mepc: u64,
    mcause: u64,
    marker: u64,
}

fn run_until_trap(backend: BackendKind, trap_latency: u64, program: &[u32]) -> TrapTaken {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.pipeline.trap_latency = trap_latency;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, program);
    store_words(&mut ctx.sim, HANDLER, &handler());
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    let mut cycle = 0;
    while ctx.sim.state.harts[0].csrs.mcause == 0 {
        ctx.run(1);
        cycle += 1;
        assert!(cycle < 5000, "{backend:?}: no trap was taken");
    }
    let csrs = &ctx.sim.state.harts[0].csrs;
    TrapTaken { cycle, mepc: csrs.mepc, mcause: csrs.mcause, marker: ctx.get_reg(T6) }
}

fn check_exception_latency(backend: BackendKind) {
    let program = [ECALL, InstructionBuilder::new().jal(X0, 0).build()];

    let at_once = run_until_trap(backend, 0, &program);
    let delayed = run_until_trap(backend, 13, &program);

    assert_eq!(at_once.mcause, 11, "{backend:?}: machine ecall");
    assert_eq!(at_once.mepc, PROGRAM_BASE);
    assert_eq!(
        delayed.cycle - at_once.cycle,
        13,
        "{backend:?}: the trap was taken 13 cycles later"
    );
}

fn check_interrupt_latency(backend: BackendKind) {
    let program = timer_program();

    let at_once = run_until_trap(backend, 0, &program);
    let delayed = run_until_trap(backend, 13, &program);

    assert_eq!(at_once.mcause, MACHINE_TIMER_INTERRUPT, "{backend:?}: machine timer interrupt");
    assert_eq!(at_once.marker, 1, "{backend:?}: the program ran to its spin first");
    assert_eq!(at_once.mepc, spin_pc(&program), "{backend:?}: the interrupt returns to the spin");
    assert_eq!(
        delayed.cycle - at_once.cycle,
        13,
        "{backend:?}: the trap was taken 13 cycles later"
    );
}

#[test]
fn an_exception_reaches_its_handler_after_the_trap_latency_o3() {
    check_exception_latency(BackendKind::OutOfOrder);
}

#[test]
fn an_exception_reaches_its_handler_after_the_trap_latency_inorder() {
    check_exception_latency(BackendKind::InOrder);
}

#[test]
fn an_interrupt_reaches_its_handler_after_the_trap_latency_o3() {
    check_interrupt_latency(BackendKind::OutOfOrder);
}

#[test]
fn an_interrupt_reaches_its_handler_after_the_trap_latency_inorder() {
    check_interrupt_latency(BackendKind::InOrder);
}

const MIP_MTIP: u64 = 1 << 7;
/// Instructions before the divide loop: the timer arming in `timer_program`
/// and the divisor.
const LOOP_START: u64 = PROGRAM_BASE + 9 * 4;
const LOOP_DIVIDES: u32 = 4;

/// Arms the timer as `timer_program` does, then divides forever, so the
/// interrupt lands while divides are in flight.
fn divide_loop() -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut program = timer_program();
    program.truncate(9);
    program.extend((0..LOOP_DIVIDES).map(|_| i().div(T3, T3, T4).build()));
    program.push(i().jal(X0, -4 * LOOP_DIVIDES as i32).build());
    program
}

/// Instructions retired between the timer interrupt becoming pending and
/// its trap being taken, and the trap's `mepc`.
fn retired_while_interrupt_pends(backend: BackendKind) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.pipeline.trap_latency = 0;
    config.system.console = Console::Quiet;
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, &divide_loop());
    store_words(&mut ctx.sim, HANDLER, &handler());
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.sim.sync_arch_regs();

    let mut retired_when_pending = None;
    for _ in 0..20_000 {
        ctx.run(1);
        let hart = &ctx.sim.state.harts[0];
        if retired_when_pending.is_none() && hart.csrs.mip & MIP_MTIP != 0 {
            retired_when_pending = Some(hart.instructions_retired);
        }
        if hart.csrs.mcause != 0 {
            let pending = retired_when_pending.expect("the interrupt was pending first");
            return (hart.instructions_retired - pending, hart.csrs.mepc);
        }
    }
    panic!("{backend:?}: no interrupt was taken");
}

/// An interrupt is taken on the instruction at the ROB head, flushing what
/// is in flight behind it, rather than waiting for it to retire (#342).
#[test]
fn an_interrupt_is_taken_on_the_instruction_at_the_rob_head() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (retired, mepc) = retired_while_interrupt_pends(backend);

        assert_eq!(retired, 0, "{backend:?}: nothing retired while the interrupt pended");
        assert!(
            (LOOP_START..LOOP_START + 4 * u64::from(LOOP_DIVIDES) + 4).contains(&mepc),
            "{backend:?}: mepc {mepc:#x} is in the divide loop"
        );
    }
}

/// The CLINT's `mtimecmp` for hart 0.
const MTIMECMP: i64 = 0x0200_4000;
/// The UART's receive register: a read takes the next byte typed.
const UART_RBR: i64 = 0x1000_0000;
/// The UART's line status register, whose bit 0 says a byte has arrived.
const UART_LSR: i32 = 5;
const TYPED: &[u8] = b"AB";
/// Cycles before the byte's read retires (with no timer) that the sweep
/// starts firing the timer: from before the read is sent to its retirement.
const SWEEP: u64 = 40;
const NEVER: u64 = u64::MAX;

/// Sets `mtimecmp` from `t5`, enables the machine timer interrupt, waits
/// for a byte to arrive, reads it into `t1` and spins.
fn read_a_byte_under_a_timer() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().sd(A4, T5, 0).build(),
        i().csrrs(X0, MIE, T2).build(),
        i().csrrs(X0, MSTATUS, T3).build(),
        i().lbu(T6, A0, UART_LSR).build(),
        i().andi(T6, T6, 1).build(),
        i().beq(T6, X0, -8).build(),
        i().lbu(T1, A0, 0).build(),
        i().jal(X0, 0).build(),
    ]
}

/// Turns the timer off and returns.
fn timer_off_handler() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![i().addi(T5, X0, -1).build(), i().sd(A4, T5, 0).build(), MRET]
}

/// The byte the program reads with the timer at cycle `fire_at`, and the
/// cycle its read retired.
fn byte_read_with_the_timer_at(backend: BackendKind, fire_at: u64) -> (u64, u64) {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = 4;
    config.system.timer = Timer::EveryCycle;
    config.system.console = Console::Captured;
    let mut ctx = TestContext::with_fast_memory(&config)
        .load_program(PROGRAM_BASE, &read_a_byte_under_a_timer());
    store_words(&mut ctx.sim, HANDLER, &timer_off_handler());
    ctx.sim.state.harts[0].csrs.mtvec = HANDLER;
    ctx.set_reg(A4, MTIMECMP as u64);
    ctx.set_reg(T5, fire_at);
    ctx.set_reg(T2, MIE_MTIE as u64);
    ctx.set_reg(T3, MSTATUS_MIE as u64);
    ctx.set_reg(A0, UART_RBR as u64);
    ctx.sim.sync_arch_regs();
    ctx.sim.send_console_input(TYPED);

    for cycle in 0..5_000 {
        ctx.run(1);
        if ctx.get_reg(T1) != 0 {
            return (ctx.get_reg(T1), cycle);
        }
    }
    panic!("{backend:?}: the byte was never read");
}

/// A device read has its side effect when it is sent, so an interrupt that
/// arrives before it retires waits for it rather than running it again
/// (#342): whenever the timer fires around the UART read, the program reads
/// the first byte typed.
#[test]
fn an_interrupt_never_repeats_a_device_read() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let (_, read_at) = byte_read_with_the_timer_at(backend, NEVER);
        for fire_at in read_at - SWEEP..=read_at {
            let (byte, _) = byte_read_with_the_timer_at(backend, fire_at);

            assert_eq!(byte, u64::from(TYPED[0]), "{backend:?}: timer at cycle {fire_at}");
        }
    }
}
