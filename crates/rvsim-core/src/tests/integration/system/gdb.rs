//! A debugger attaches over a loopback socket with GDB's remote protocol,
//! sets a breakpoint, runs to it, reads and writes memory and registers,
//! steps one instruction, and runs the guest to its exit.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use crate::config::{BackendKind, Config, Console};
use crate::system::gdb::Outcome;
use crate::system::simulator::Simulator;
use crate::tests::support::builder::instruction::{InstructionBuilder, M5_FAIL};
use crate::tests::support::harness::TestContext;

const PROGRAM_BASE: u64 = 0x8000_0000;
const T0: u32 = 5;
const T2: u32 = 7;
const A1: u32 = 11;
const JAL_SELF: u32 = 0x0000_006f;
/// Where the program stores `t0`: `auipc t2` at `PROGRAM_BASE + 4` plus `0x100`.
const STORED_AT: u64 = PROGRAM_BASE + 4 + 0x100;
/// The `addi a1` before the exit, where the breakpoint goes.
const BREAK_AT: u64 = PROGRAM_BASE + 12;

/// Stores 7, sets the exit code 3, exits.
fn program() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 7).build(),
        i().auipc(T2, 0).build(),
        i().sd(T2, T0, 0x100).build(),
        i().addi(A1, 0, 3).build(),
        M5_FAIL,
        JAL_SELF,
    ]
}

/// A debugger's side of the connection: sends packets and reads replies,
/// acknowledging until no-ack mode is on.
struct Debugger {
    stream: TcpStream,
    acks: bool,
}

impl Debugger {
    fn connect(address: std::net::SocketAddr) -> Self {
        let stream = TcpStream::connect(address).expect("connect");
        let mut gdb = Self { stream, acks: true };
        assert!(gdb.ask("qSupported:swbreak+;hwbreak+").contains("QStartNoAckMode+"));
        assert_eq!(gdb.ask("QStartNoAckMode"), "OK");
        gdb.acks = false;
        gdb
    }

    fn send(&mut self, payload: &str) {
        let sum = payload.bytes().fold(0u8, u8::wrapping_add);
        self.stream.write_all(format!("${payload}#{sum:02x}").as_bytes()).expect("send");
    }

    /// The next reply's payload.
    fn receive(&mut self) -> String {
        let mut bytes = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            self.stream.read_exact(&mut byte).expect("a reply");
            match byte[0] {
                b'+' | b'-' if bytes.is_empty() => {}
                b'$' => bytes.clear(),
                b'#' => {
                    let mut sum = [0u8; 2];
                    self.stream.read_exact(&mut sum).expect("a checksum");
                    if self.acks {
                        self.stream.write_all(b"+").expect("ack");
                    }
                    return String::from_utf8(bytes).expect("ascii");
                }
                other => bytes.push(other),
            }
        }
    }

    fn ask(&mut self, payload: &str) -> String {
        self.send(payload);
        self.receive()
    }
}

fn context(backend: BackendKind, harts: usize) -> TestContext {
    let mut config = Config::default();
    config.pipeline.backend = backend;
    config.system.console = Console::Quiet;
    config.system.hart_count = harts;
    TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &program())
}

/// Serves one debugger whose script is `client`, and returns what the
/// script collected and how the session ended.
fn debug_session<T: Send + 'static>(
    sim: &mut Simulator,
    client: impl FnOnce(Debugger) -> T + Send + 'static,
) -> (T, Outcome) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the port");
    let script = thread::spawn(move || client(Debugger::connect(address)));
    let (stream, _) = listener.accept().expect("the debugger connects");
    let outcome = sim.serve_gdb(stream, || true).expect("the session runs");
    (script.join().expect("the debugger's script finishes"), outcome)
}

#[test]
fn a_debugger_breaks_reads_writes_steps_and_sees_the_exit() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = context(backend, 1);

        let (seen, outcome) = debug_session(&mut ctx.sim, |mut gdb| {
            vec![
                gdb.ask("qfThreadInfo"),
                gdb.ask(&format!("Z0,{BREAK_AT:x},4")),
                gdb.ask("c"),
                gdb.ask(&format!("m{STORED_AT:x},8")),
                gdb.ask("p5"),
                gdb.ask("p20"),
                gdb.ask(&format!("M{STORED_AT:x},8:2a00000000000000")),
                gdb.ask(&format!("m{STORED_AT:x},8")),
                gdb.ask("P5=0900000000000000"),
                gdb.ask("p5"),
                gdb.ask("s"),
                gdb.ask("p20"),
                gdb.ask("c"),
            ]
        });

        assert_eq!(seen[0], "m1", "{backend:?}: one hart, one thread");
        assert_eq!(seen[1], "OK", "{backend:?}: the breakpoint is set");
        assert!(
            seen[2].starts_with("T05thread:1;")
                && seen[2].contains("20:0c00008000000000;swbreak:;"),
            "{backend:?}: stopped at the breakpoint: {}",
            seen[2]
        );
        assert_eq!(seen[3], "0700000000000000", "{backend:?}: the store before it is visible");
        assert_eq!(seen[4], "0700000000000000", "{backend:?}: t0 holds 7");
        assert_eq!(seen[5], "0c00008000000000", "{backend:?}: pc is the breakpoint");
        assert_eq!(seen[6], "OK", "{backend:?}: memory is written");
        assert_eq!(seen[7], "2a00000000000000", "{backend:?}: and read back");
        assert_eq!(seen[8], "OK", "{backend:?}: t0 is written");
        assert_eq!(seen[9], "0900000000000000", "{backend:?}: and read back");
        assert!(
            seen[10].starts_with("T05thread:1;"),
            "{backend:?}: the step stopped: {}",
            seen[10]
        );
        assert_eq!(seen[11], "1000008000000000", "{backend:?}: one instruction later");
        assert_eq!(seen[12], "W03", "{backend:?}: the guest exited with 3");
        assert_eq!(outcome, Outcome::Exited(3), "{backend:?}");
    }
}

#[test]
fn a_register_written_while_stopped_is_what_the_program_computes_with() {
    for backend in [BackendKind::InOrder, BackendKind::OutOfOrder] {
        let mut ctx = context(backend, 1);

        let (seen, outcome) = debug_session(&mut ctx.sim, |mut gdb| {
            // Stop before the store, change t0, and let the store run.
            vec![
                gdb.ask(&format!("Z1,{:x},4", PROGRAM_BASE + 8)),
                gdb.ask("c"),
                gdb.ask("P5=0000000000000000"),
                gdb.ask(&format!("Z1,{BREAK_AT:x},4")),
                gdb.ask("c"),
                gdb.ask(&format!("m{STORED_AT:x},8")),
                gdb.ask("D"),
            ]
        });

        assert!(seen[1].contains("hwbreak:;"), "{backend:?}: {}", seen[1]);
        assert_eq!(seen[2], "OK", "{backend:?}");
        assert!(seen[4].starts_with("T05thread:1;"), "{backend:?}: {}", seen[4]);
        assert_eq!(seen[5], "0000000000000000", "{backend:?}: the store wrote the new t0, zero");
        assert_eq!(seen[6], "OK", "{backend:?}");
        assert_eq!(outcome, Outcome::Detached, "{backend:?}");
    }
}

/// Independent `addi`s an out-of-order core retires several of a cycle,
/// then the exit.
fn independent_adds() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(T0, 0, 1).build(),
        i().addi(6, 0, 2).build(),
        i().addi(T2, 0, 3).build(),
        i().addi(28, 0, 4).build(),
        i().addi(A1, 0, 3).build(),
        M5_FAIL,
        JAL_SELF,
    ]
}

#[test]
fn a_breakpoint_inside_a_commit_group_stops_before_its_instruction() {
    let mut config = Config::default();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.system.console = Console::Quiet;
    let mut ctx =
        TestContext::new_with_config(&config).load_program(PROGRAM_BASE, &independent_adds());
    let third = PROGRAM_BASE + 8;

    let (seen, outcome) = debug_session(&mut ctx.sim, move |mut gdb| {
        vec![
            gdb.ask(&format!("Z0,{third:x},4")),
            gdb.ask("c"),
            gdb.ask("p6"),
            gdb.ask("p7"),
            gdb.ask("D"),
        ]
    });

    assert!(
        seen[1].contains(&format!("20:{};", hex_le(third))),
        "stopped at the third add: {}",
        seen[1]
    );
    assert_eq!(seen[2], "0200000000000000", "the add before it retired");
    assert_eq!(seen[3], "0000000000000000", "the add at the breakpoint did not");
    assert_eq!(outcome, Outcome::Detached);
}

fn hex_le(value: u64) -> String {
    value.to_le_bytes().iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn every_hart_is_a_thread() {
    let mut ctx = context(BackendKind::InOrder, 2);

    let (seen, outcome) = debug_session(&mut ctx.sim, |mut gdb| {
        vec![
            gdb.ask("qfThreadInfo"),
            gdb.ask("qsThreadInfo"),
            gdb.ask("Hg2"),
            gdb.ask("qC"),
            gdb.ask("T2"),
            gdb.ask("T3"),
            gdb.ask(&format!("p{:x}", 65 + 0xf14)),
            gdb.ask("qThreadExtraInfo,2"),
            gdb.ask("vKill;1"),
        ]
    });

    assert_eq!(seen[0], "m1,2");
    assert_eq!(seen[1], "l");
    assert_eq!(seen[2], "OK");
    assert_eq!(seen[3], "QC2");
    assert_eq!(seen[4], "OK");
    assert_eq!(seen[5], "E01", "there is no third thread");
    assert_eq!(seen[6], "0100000000000000", "mhartid on the second hart");
    assert_eq!(seen[7], "686172742031", "\"hart 1\"");
    assert_eq!(seen[8], "OK");
    assert_eq!(outcome, Outcome::Killed);
}
