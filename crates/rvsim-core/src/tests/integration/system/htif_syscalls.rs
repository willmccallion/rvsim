//! The HTIF syscall proxy: a bare-metal program hands the host an
//! eight-word block through `tohost`, polls `fromhost`, and finds the
//! result in the block, as it would under spike's proxy.

use crate::config::{BackendKind, Config, Console};
use crate::system::simulator::{StopAt, StopReason};
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const TOHOST: u64 = PROGRAM_BASE + 0x1000;
const T0: u32 = 5;
const T1: u32 = 6;
const T2: u32 = 7;
const A0: u32 = 10;
const A1: u32 = 11;
const A2: u32 = 12;
const FENCE: u32 = 0x0ff0_000f;
const SYS_EXIT: i32 = 93;
const SYS_GETMAINVARS: i32 = 2011;

/// `t0` = tohost, `t1` = the syscall block, `t2` = a buffer, each a page
/// above the program; `auipc` keeps the addresses relative to it.
fn addresses() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T0, 1).build(),
        i().auipc(T1, 2).build(),
        i().addi(T1, T1, -4).build(),
        i().auipc(T2, 3).build(),
        i().addi(T2, T2, -12).build(),
    ]
}

/// Writes `number` and `args` into the block, hands it to the host and
/// waits for `fromhost`, then clears it.
fn syscall(number: i32, args: &[(u32, Option<i32>)]) -> Vec<u32> {
    let i = InstructionBuilder::new;
    let mut code = vec![i().addi(A0, 0, number).build(), i().sd(T1, A0, 0).build()];
    for (slot, (reg, immediate)) in args.iter().enumerate() {
        let offset = 8 * (slot as i32 + 1);
        match immediate {
            Some(value) => {
                code.push(i().addi(A0, 0, *value).build());
                code.push(i().sd(T1, A0, offset).build());
            }
            None => code.push(i().sd(T1, *reg, offset).build()),
        }
    }
    code.push(FENCE);
    code.push(i().sd(T0, T1, 0).build());
    code.push(i().ld(A1, T0, 8).build());
    code.push(i().beq(A1, 0, -4).build());
    code.push(i().sd(T0, 0, 8).build());
    code
}

fn run(backend: BackendKind, program: &[u32], args: &[&str]) -> StopReason {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    let mut tc = TestContext::new_with_config(&config).load_program(PROGRAM_BASE, program);
    let exit_signal = std::sync::Arc::clone(&tc.sim.state.exit_signal);
    tc.sim.state.uncore.add_htif(
        TOHOST,
        &exit_signal,
        args.iter().map(ToString::to_string).collect(),
    );
    tc.sim.run_to(&StopAt { cycles: Some(50_000), ..StopAt::default() }).expect("the program runs")
}

/// `getmainvars` writes `argc` first; the program exits with it through
/// `tohost`, as `(argc << 1) | 1`.
#[test]
fn getmainvars_hands_the_program_its_arguments() {
    let i = InstructionBuilder::new;
    let mut program = addresses();
    program.extend(syscall(SYS_GETMAINVARS, &[(T2, None), (0, Some(256))]));
    program.extend([
        i().ld(A2, T2, 0).build(),
        i().add(A2, A2, A2).build(),
        i().addi(A2, A2, 1).build(),
        i().sd(T0, A2, 0).build(),
    ]);
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &program, &["prog", "one"]);

        assert_eq!(stop, StopReason::Exited(2), "{backend:?}");
    }
}

/// An `exit` through the block ends the run with its code.
#[test]
fn exit_through_the_block_ends_the_run() {
    let mut program = addresses();
    program.extend(syscall(SYS_EXIT, &[(0, Some(5))]));
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let stop = run(backend, &program, &["prog"]);

        assert_eq!(stop, StopReason::Exited(5), "{backend:?}");
    }
}
