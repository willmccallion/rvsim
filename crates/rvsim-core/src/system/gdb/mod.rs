//! A GDB remote serial protocol server: GDB connects over TCP and drives
//! the simulator with breakpoints, single steps, and register and memory
//! access, each hart one of its threads.
//!
//! Breakpoints are stops at a PC, so no guest code is patched and they
//! work in ROM and across page tables; memory is read and written through
//! the selected hart's current translation, as the hart itself would see
//! it; a step retires exactly one instruction on the selected hart. Every
//! stop is a [`Simulator::drain`]: the pipelines are emptied and committed
//! stores reach memory, as a hardware halt would leave the core, so the
//! debugger sees architectural state and a stop costs simulated cycles.

mod packet;
mod target;

use std::collections::BTreeMap;
use std::net::TcpStream;

use crate::common::{CoreId, SimError};
use crate::isa::csr::CsrAddr;
use crate::isa::reg::RegIdx;
use crate::system::simulator::{Simulator, StopAt, StopReason};
use packet::{Incoming, Transport};
use target::{FIRST_CSR_REGNUM, FIRST_FP_REGNUM, PC_REGNUM};

/// How a debugging session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The debugger detached (or hung up); the run may go on without it.
    Detached,
    /// The debugger killed the run.
    Killed,
    /// The guest exited, with this code, while the debugger watched.
    Exited(u64),
    /// The host asked to stop serving.
    Interrupted,
}

/// GDB's signal numbers in stop replies.
const SIGINT: u8 = 2;
const SIGTRAP: u8 = 5;
/// The largest packet we accept, as told to GDB.
const PACKET_SIZE: usize = 0x4000;
/// GDB's "no such register" and "cannot access memory" errors.
const E_REGISTER: &[u8] = b"E01";
const E_MEMORY: &[u8] = b"E0e";

/// The kind of breakpoint GDB set, named in the stop reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Breakpoint {
    Software,
    Hardware,
}

/// Whether the session goes on after a packet.
enum Flow {
    Continue,
    End(Outcome),
}

/// What a resume asks of one hart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Continue,
    Step,
}

/// Serves `stream` until the debugger detaches, kills the run, or the
/// guest exits; `keep_going` is the host's own say on going on.
///
/// # Errors
///
/// [`SimError::Debugger`] when the connection fails, or a tick's error.
pub fn serve(
    sim: &mut Simulator,
    stream: TcpStream,
    mut keep_going: impl FnMut() -> bool,
) -> Result<Outcome, SimError> {
    sim.drain();
    let mut session = Session {
        sim,
        transport: Transport::new(stream),
        selected: 0,
        breakpoints: BTreeMap::new(),
    };
    loop {
        match session.transport.receive().map_err(debugger)? {
            Incoming::Closed => return Ok(Outcome::Detached),
            Incoming::Idle if !keep_going() => return Ok(Outcome::Interrupted),
            Incoming::Idle | Incoming::Interrupt => {}
            Incoming::Packet(payload) => {
                if let Flow::End(outcome) = session.handle(&payload, &mut keep_going)? {
                    return Ok(outcome);
                }
            }
        }
    }
}

const fn debugger(source: std::io::Error) -> SimError {
    SimError::Debugger { source }
}

struct Session<'a> {
    sim: &'a mut Simulator,
    transport: Transport,
    /// The hart register and memory packets address: GDB's selected thread.
    selected: usize,
    breakpoints: BTreeMap<u64, Breakpoint>,
}

impl Session<'_> {
    fn handle(
        &mut self,
        payload: &[u8],
        keep_going: &mut impl FnMut() -> bool,
    ) -> Result<Flow, SimError> {
        let Some((&head, rest)) = payload.split_first() else {
            self.send(b"")?;
            return Ok(Flow::Continue);
        };
        let reply: Vec<u8> = match head {
            b'?' => self.stop_reply(self.selected, SIGTRAP),
            b'!' => b"OK".to_vec(),
            b'g' => self.read_general_registers(),
            b'G' => self.write_general_registers(rest),
            b'p' => self.read_register(rest),
            b'P' => self.write_register(rest),
            b'm' => self.read_memory(rest),
            b'M' => self.write_memory_hex(rest),
            b'X' => self.write_memory_binary(rest),
            b'c' | b's' => {
                if let Some(addr) = parse_hex_u64(rest).filter(|_| !rest.is_empty()) {
                    self.sim.set_pc(self.selected, addr);
                }
                let mut actions = vec![Action::Continue; self.sim.hart_count()];
                if head == b's' {
                    actions[self.selected] = Action::Step;
                }
                return self.resume(&actions, keep_going);
            }
            b'v' => return self.v_packet(rest, keep_going),
            b'Z' | b'z' => self.breakpoint(head == b'Z', rest),
            b'H' => self.select_thread(rest),
            b'T' => self.thread_alive(rest),
            b'q' => self.query(rest),
            b'Q' if rest == b"StartNoAckMode" => {
                self.send(b"OK")?;
                self.transport.stop_acknowledging();
                return Ok(Flow::Continue);
            }
            b'k' => return Ok(Flow::End(Outcome::Killed)),
            b'D' => {
                self.send(b"OK")?;
                return Ok(Flow::End(Outcome::Detached));
            }
            _ => Vec::new(),
        };
        self.send(&reply)?;
        Ok(Flow::Continue)
    }

    fn send(&mut self, payload: &[u8]) -> Result<(), SimError> {
        self.transport.send(payload).map_err(debugger)
    }

    /// `T<signal>thread:<id>;<pc regnum>:<pc>;` with the breakpoint kind
    /// when the hart sits on one.
    fn stop_reply(&self, hart: usize, signal: u8) -> Vec<u8> {
        let pc = self.sim.hart(hart).pc;
        let mut reply =
            format!("T{signal:02x}thread:{:x};{PC_REGNUM:x}:{};", hart + 1, hex_le(pc, 8));
        match self.breakpoints.get(&pc) {
            Some(Breakpoint::Software) if signal == SIGTRAP => reply.push_str("swbreak:;"),
            Some(Breakpoint::Hardware) if signal == SIGTRAP => reply.push_str("hwbreak:;"),
            _ => {}
        }
        reply.into_bytes()
    }

    fn read_general_registers(&self) -> Vec<u8> {
        let hart = self.sim.hart(self.selected);
        let mut reply = String::with_capacity(33 * 16);
        for n in 0..32u8 {
            reply.push_str(&hex_le(hart.regs.read(RegIdx::new(n)), 8));
        }
        reply.push_str(&hex_le(hart.pc, 8));
        reply.into_bytes()
    }

    /// `G`: the integer registers and `pc`, as `g` lists them; whatever GDB
    /// appends for the registers `g` left out is ignored.
    fn write_general_registers(&mut self, hex: &[u8]) -> Vec<u8> {
        const WANTED: usize = 33 * 8;
        let Some(bytes) = hex.get(..WANTED * 2).and_then(parse_hex_bytes) else {
            return E_REGISTER.to_vec();
        };
        let hart = self.selected;
        for n in 1..32usize {
            self.sim.hart_mut(hart).regs.write(RegIdx::new(n as u8), word_at(&bytes, n));
        }
        self.sim.set_pc(hart, word_at(&bytes, 32));
        self.sim.sync_arch_regs();
        b"OK".to_vec()
    }

    fn read_register(&mut self, hex: &[u8]) -> Vec<u8> {
        let Some(regnum) = parse_hex_u32(hex) else { return E_REGISTER.to_vec() };
        match self.register(regnum) {
            Some((value, bytes)) => hex_le(value, bytes).into_bytes(),
            None => E_REGISTER.to_vec(),
        }
    }

    /// Register `regnum`'s value and width in bytes on the selected hart.
    fn register(&mut self, regnum: u32) -> Option<(u64, usize)> {
        let hart = self.selected;
        if regnum < PC_REGNUM {
            return Some((self.sim.hart(hart).regs.read(RegIdx::new(regnum as u8)), 8));
        }
        if regnum == PC_REGNUM {
            return Some((self.sim.hart(hart).pc, 8));
        }
        if (FIRST_FP_REGNUM..FIRST_CSR_REGNUM).contains(&regnum) {
            let f = RegIdx::new((regnum - FIRST_FP_REGNUM) as u8);
            return Some((self.sim.hart(hart).regs.read_f(f), 8));
        }
        let addr = regnum.checked_sub(FIRST_CSR_REGNUM)?;
        let value = self.sim.read_csr(hart, CsrAddr::from_u32(addr))?;
        Some((value, csr_bytes(addr)))
    }

    fn write_register(&mut self, rest: &[u8]) -> Vec<u8> {
        let Some(eq) = rest.iter().position(|&b| b == b'=') else { return E_REGISTER.to_vec() };
        let (Some(regnum), Some(bytes)) =
            (parse_hex_u32(&rest[..eq]), parse_hex_bytes(&rest[eq + 1..]))
        else {
            return E_REGISTER.to_vec();
        };
        let mut word = [0u8; 8];
        for (slot, byte) in word.iter_mut().zip(&bytes) {
            *slot = *byte;
        }
        let value = u64::from_le_bytes(word);
        let hart = self.selected;
        let written = if regnum < PC_REGNUM {
            if regnum != 0 {
                self.sim.hart_mut(hart).regs.write(RegIdx::new(regnum as u8), value);
            }
            true
        } else if regnum == PC_REGNUM {
            self.sim.set_pc(hart, value);
            true
        } else if (FIRST_FP_REGNUM..FIRST_CSR_REGNUM).contains(&regnum) {
            self.sim
                .hart_mut(hart)
                .regs
                .write_f(RegIdx::new((regnum - FIRST_FP_REGNUM) as u8), value);
            true
        } else {
            regnum
                .checked_sub(FIRST_CSR_REGNUM)
                .is_some_and(|addr| self.sim.write_csr(hart, CsrAddr::from_u32(addr), value))
        };
        if !written {
            return E_REGISTER.to_vec();
        }
        self.sim.sync_arch_regs();
        b"OK".to_vec()
    }

    fn read_memory(&mut self, rest: &[u8]) -> Vec<u8> {
        let Some((addr, len)) = parse_addr_len(rest) else { return E_MEMORY.to_vec() };
        let Ok(len) = usize::try_from(len) else { return E_MEMORY.to_vec() };
        if len > PACKET_SIZE / 2 {
            return E_MEMORY.to_vec();
        }
        let Some(core) = self.core_of(self.selected) else { return E_MEMORY.to_vec() };
        self.sim
            .state
            .core_ctx(core)
            .read_now(addr, len)
            .map_or_else(|_| E_MEMORY.to_vec(), |bytes| hex_bytes(&bytes).into_bytes())
    }

    /// `M addr,len:hex`.
    fn write_memory_hex(&mut self, rest: &[u8]) -> Vec<u8> {
        let Some(colon) = rest.iter().position(|&b| b == b':') else { return E_MEMORY.to_vec() };
        let Some(bytes) = parse_hex_bytes(&rest[colon + 1..]) else { return E_MEMORY.to_vec() };
        self.write_memory(&rest[..colon], &bytes)
    }

    /// `X addr,len:binary`, the escaping already undone.
    fn write_memory_binary(&mut self, rest: &[u8]) -> Vec<u8> {
        let Some(colon) = rest.iter().position(|&b| b == b':') else { return E_MEMORY.to_vec() };
        self.write_memory(&rest[..colon], &rest[colon + 1..])
    }

    fn write_memory(&mut self, addr_len: &[u8], bytes: &[u8]) -> Vec<u8> {
        let Some((addr, len)) = parse_addr_len(addr_len) else { return E_MEMORY.to_vec() };
        if bytes.len() as u64 != len {
            return E_MEMORY.to_vec();
        }
        if bytes.is_empty() {
            return b"OK".to_vec();
        }
        let Some(core) = self.core_of(self.selected) else { return E_MEMORY.to_vec() };
        if self.sim.state.core_ctx(core).write_now(addr, bytes).is_err() {
            return E_MEMORY.to_vec();
        }
        b"OK".to_vec()
    }

    fn core_of(&self, hart: usize) -> Option<usize> {
        let hart_id = self.sim.state.harts.get(hart)?.hart_id;
        self.sim.state.topology.core_of_hart(hart_id).map(CoreId::as_index)
    }

    fn breakpoint(&mut self, insert: bool, rest: &[u8]) -> Vec<u8> {
        let mut fields = rest.split(|&b| b == b',');
        let (Some(kind), Some(addr)) = (fields.next(), fields.next()) else {
            return E_MEMORY.to_vec();
        };
        let kind = match kind {
            b"0" => Breakpoint::Software,
            b"1" => Breakpoint::Hardware,
            _ => return Vec::new(),
        };
        let Some(addr) = parse_hex_u64(addr) else { return E_MEMORY.to_vec() };
        if insert {
            let _ = self.breakpoints.insert(addr, kind);
        } else {
            let _ = self.breakpoints.remove(&addr);
        }
        b"OK".to_vec()
    }

    fn select_thread(&mut self, rest: &[u8]) -> Vec<u8> {
        let Some((_, id)) = rest.split_first() else { return E_REGISTER.to_vec() };
        match parse_thread(id) {
            Some(Thread::Hart(hart)) if hart < self.sim.hart_count() => {
                self.selected = hart;
                b"OK".to_vec()
            }
            Some(Thread::Any) => b"OK".to_vec(),
            _ => E_REGISTER.to_vec(),
        }
    }

    fn thread_alive(&self, id: &[u8]) -> Vec<u8> {
        match parse_thread(id) {
            Some(Thread::Hart(hart)) if hart < self.sim.hart_count() => b"OK".to_vec(),
            _ => E_REGISTER.to_vec(),
        }
    }

    fn query(&self, rest: &[u8]) -> Vec<u8> {
        if rest.starts_with(b"Supported") {
            return format!(
                "PacketSize={PACKET_SIZE:x};QStartNoAckMode+;qXfer:features:read+;vContSupported+;swbreak+;hwbreak+"
            )
            .into_bytes();
        }
        if rest == b"Attached" {
            return b"1".to_vec();
        }
        if rest == b"C" {
            return format!("QC{:x}", self.selected + 1).into_bytes();
        }
        if rest == b"fThreadInfo" {
            let ids: Vec<String> =
                (1..=self.sim.hart_count()).map(|id| format!("{id:x}")).collect();
            return format!("m{}", ids.join(",")).into_bytes();
        }
        if rest == b"sThreadInfo" {
            return b"l".to_vec();
        }
        if let Some(id) = rest.strip_prefix(b"ThreadExtraInfo,") {
            return match parse_thread(id) {
                Some(Thread::Hart(hart)) => {
                    hex_bytes(format!("hart {hart}").as_bytes()).into_bytes()
                }
                _ => E_REGISTER.to_vec(),
            };
        }
        if let Some(range) = rest.strip_prefix(b"Xfer:features:read:target.xml:") {
            return target_chunk(range);
        }
        if rest.starts_with(b"Symbol:") {
            return b"OK".to_vec();
        }
        Vec::new()
    }

    fn v_packet(
        &mut self,
        rest: &[u8],
        keep_going: &mut impl FnMut() -> bool,
    ) -> Result<Flow, SimError> {
        if rest == b"Cont?" {
            self.send(b"vCont;c;C;s;S")?;
            return Ok(Flow::Continue);
        }
        if let Some(actions) = rest.strip_prefix(b"Cont;") {
            let actions = self.cont_actions(actions);
            return self.resume(&actions, keep_going);
        }
        if rest.starts_with(b"Kill") {
            self.send(b"OK")?;
            return Ok(Flow::End(Outcome::Killed));
        }
        self.send(b"")?;
        Ok(Flow::Continue)
    }

    /// The action for each hart from a `vCont` list: the first naming the
    /// hart, else the first naming no hart, else to continue.
    fn cont_actions(&self, list: &[u8]) -> Vec<Action> {
        let mut actions = vec![None; self.sim.hart_count()];
        let mut default = None;
        for item in list.split(|&b| b == b';') {
            let mut parts = item.split(|&b| b == b':');
            let action = match parts.next().and_then(|a| a.first()) {
                Some(b's' | b'S') => Action::Step,
                _ => Action::Continue,
            };
            match parts.next().and_then(parse_thread) {
                Some(Thread::Hart(hart)) => {
                    if let Some(slot) = actions.get_mut(hart) {
                        let _ = slot.get_or_insert(action);
                    }
                }
                _ => {
                    let _ = default.get_or_insert(action);
                }
            }
        }
        actions.into_iter().map(|action| action.or(default).unwrap_or(Action::Continue)).collect()
    }

    /// Runs as `actions` ask and reports where the run stopped.
    fn resume(
        &mut self,
        actions: &[Action],
        keep_going: &mut impl FnMut() -> bool,
    ) -> Result<Flow, SimError> {
        let mut host_stopped = false;
        let reason = {
            let Session { sim, transport, breakpoints, .. } = &mut *self;
            let mut going = || {
                if transport.interrupted() {
                    return false;
                }
                if keep_going() {
                    return true;
                }
                host_stopped = true;
                false
            };
            run(sim, breakpoints, actions, &mut going)?
        };
        if host_stopped {
            return Ok(Flow::End(Outcome::Interrupted));
        }
        if let StopReason::Exited(code) = reason {
            self.send(format!("W{:02x}", code & 0xff).as_bytes())?;
            return Ok(Flow::End(Outcome::Exited(code)));
        }
        self.sim.drain();
        let signal = match reason {
            StopReason::Pc { hart } => {
                self.selected = hart;
                SIGTRAP
            }
            StopReason::Cancelled => SIGINT,
            StopReason::Exited(_)
            | StopReason::Cycles
            | StopReason::Instructions
            | StopReason::GuestBreak { .. }
            | StopReason::ConsoleOutput => SIGTRAP,
        };
        let reply = self.stop_reply(self.selected, signal);
        self.send(&reply)?;
        Ok(Flow::Continue)
    }
}

/// Runs until a stepped hart has moved, a hart arrives at a breakpoint,
/// the guest exits or asks to stop, or `going` says to stop.
///
/// A hart that starts on a breakpoint has to leave it before that
/// breakpoint counts again, so continuing from a stop moves on; while a
/// hart steps or waits to leave, the run goes a cycle at a time.
fn run(
    sim: &mut Simulator,
    breakpoints: &BTreeMap<u64, Breakpoint>,
    actions: &[Action],
    going: &mut impl FnMut() -> bool,
) -> Result<StopReason, SimError> {
    let starts: Vec<Start> = (0..sim.hart_count())
        .map(|hart| Start::of(sim, hart, breakpoints.contains_key(&sim.hart(hart).pc)))
        .collect();
    for (hart, action) in actions.iter().enumerate() {
        sim.set_single_step(hart, *action == Action::Step);
    }
    let result = run_from(sim, breakpoints, actions, &starts, going);
    for hart in 0..sim.hart_count() {
        sim.set_single_step(hart, false);
    }
    result
}

/// Where a hart was when a resume began.
struct Start {
    pc: u64,
    instructions_retired: u64,
    on_breakpoint: bool,
}

impl Start {
    fn of(sim: &Simulator, hart: usize, on_breakpoint: bool) -> Self {
        let hart = sim.hart(hart);
        Self { pc: hart.pc, instructions_retired: hart.instructions_retired, on_breakpoint }
    }

    /// Whether the hart has retired an instruction or taken a trap since.
    fn moved(&self, sim: &Simulator, hart: usize) -> bool {
        let hart = sim.hart(hart);
        hart.pc != self.pc || hart.instructions_retired != self.instructions_retired
    }
}

fn run_from(
    sim: &mut Simulator,
    breakpoints: &BTreeMap<u64, Breakpoint>,
    actions: &[Action],
    starts: &[Start],
    going: &mut impl FnMut() -> bool,
) -> Result<StopReason, SimError> {
    let one_cycle = StopAt { cycles: Some(1), guest_breaks: true, ..StopAt::default() };
    loop {
        let cycle_by_cycle = starts.iter().enumerate().any(|(hart, start)| {
            actions[hart] == Action::Step || (start.on_breakpoint && !start.moved(sim, hart))
        });
        if !cycle_by_cycle {
            let stop = StopAt {
                pcs: breakpoints.keys().copied().collect(),
                guest_breaks: true,
                ..StopAt::default()
            };
            return sim.run_to_with(&stop, going);
        }
        match sim.run_to_with(&one_cycle, &mut *going)? {
            StopReason::Cycles => {}
            other => return Ok(other),
        }
        for (hart, start) in starts.iter().enumerate() {
            let moved = start.moved(sim, hart);
            if actions[hart] == Action::Step && moved {
                return Ok(StopReason::Pc { hart });
            }
            if breakpoints.contains_key(&sim.hart(hart).pc) && (moved || !start.on_breakpoint) {
                return Ok(StopReason::Pc { hart });
            }
        }
    }
}

/// The bytes of a CSR as the target description declares it.
const fn csr_bytes(addr: u32) -> usize {
    if addr <= 3 { 4 } else { 8 }
}

/// A slice of the target description for `qXfer:features:read`, with
/// `l` for the last piece and `m` for one with more to come.
fn target_chunk(range: &[u8]) -> Vec<u8> {
    let xml = target::description();
    let Some((offset, length)) = parse_addr_len(range) else { return E_REGISTER.to_vec() };
    let (Ok(offset), Ok(length)) = (usize::try_from(offset), usize::try_from(length)) else {
        return E_REGISTER.to_vec();
    };
    let start = offset.min(xml.len());
    let end = start.saturating_add(length).min(xml.len());
    let mut reply = Vec::with_capacity(end - start + 1);
    reply.push(if end == xml.len() { b'l' } else { b'm' });
    reply.extend_from_slice(&xml.as_bytes()[start..end]);
    reply
}

/// A thread in a packet: a hart, or GDB's "any" (0) and "all" (-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Thread {
    Any,
    Hart(usize),
}

fn parse_thread(id: &[u8]) -> Option<Thread> {
    let text = std::str::from_utf8(id).ok()?;
    let text =
        text.strip_prefix('p').map_or(text, |rest| rest.split_once('.').map_or(rest, |(_, t)| t));
    if text == "-1" || text == "0" {
        return Some(Thread::Any);
    }
    let id = usize::from_str_radix(text, 16).ok()?;
    id.checked_sub(1).map(Thread::Hart)
}

/// `addr,len`, both hex.
fn parse_addr_len(text: &[u8]) -> Option<(u64, u64)> {
    let mut parts = text.split(|&b| b == b',');
    let addr = parse_hex_u64(parts.next()?)?;
    let len = parse_hex_u64(parts.next()?)?;
    Some((addr, len))
}

fn parse_hex_u64(text: &[u8]) -> Option<u64> {
    u64::from_str_radix(std::str::from_utf8(text).ok()?, 16).ok()
}

fn parse_hex_u32(text: &[u8]) -> Option<u32> {
    u32::from_str_radix(std::str::from_utf8(text).ok()?, 16).ok()
}

/// Pairs of hex digits to bytes.
fn parse_hex_bytes(text: &[u8]) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// The `n`th little-endian 64-bit word of `bytes`.
fn word_at(bytes: &[u8], n: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[n * 8..n * 8 + 8]);
    u64::from_le_bytes(word)
}

/// `value`'s low `bytes` bytes, little-endian, as hex digits: the order
/// GDB expects a register in.
fn hex_le(value: u64, bytes: usize) -> String {
    hex_bytes(&value.to_le_bytes()[..bytes])
}

fn hex_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_are_hex_little_endian() {
        assert_eq!(hex_le(0x8000_0004, 8), "0400008000000000");
        assert_eq!(hex_le(0x1f, 4), "1f000000");
        assert_eq!(parse_hex_bytes(b"0400008000000000").map(|b| word_at(&b, 0)), Some(0x8000_0004));
    }

    #[test]
    fn thread_ids_are_one_based_harts_with_any_and_all() {
        assert_eq!(parse_thread(b"1"), Some(Thread::Hart(0)));
        assert_eq!(parse_thread(b"a"), Some(Thread::Hart(9)));
        assert_eq!(parse_thread(b"0"), Some(Thread::Any));
        assert_eq!(parse_thread(b"-1"), Some(Thread::Any));
        assert_eq!(parse_thread(b"p1.2"), Some(Thread::Hart(1)));
    }

    #[test]
    fn the_target_description_is_served_in_pieces() {
        let whole = target::description();
        let first = target_chunk(b"0,10");
        let last = target_chunk(format!("{:x},1000", whole.len() - 5).as_bytes());

        assert_eq!(first[0], b'm');
        assert_eq!(&first[1..], &whole.as_bytes()[..16]);
        assert_eq!(last[0], b'l');
        assert_eq!(last.len(), 6);
    }
}
