//! Host-Target Interface (HTIF) device.
//!
//! The `tohost`/`fromhost` protocol riscv-tests, spike's proxy (fesvr)
//! and Chipyard's Verilator harness speak, served as fesvr serves it so
//! one ELF behaves the same on all three. A `tohost` write names a device
//! in its top byte and a command in the next; device 0's command 0 is the
//! syscall proxy: an odd payload ends the run with the code above its low
//! bit (`1` is success), and an even one is the address of an eight-word
//! block holding a syscall number and its arguments, which the host
//! performs against its own files and console before writing the result
//! into the block's first word and answering through `fromhost`. The
//! target polls `fromhost`, which this device serves from the word after
//! `tohost`, and clears it.
//!
//! The device occupies the 16 bytes at the `tohost` ELF symbol, with
//! `fromhost` expected at the next word as every HTIF target lays it out.
//! It shares the exit signal with `SysCon` so the run ends without extra
//! plumbing.

use serde::{Deserialize, Serialize};

use crate::common::{LineAddr, PhysAddr};
use crate::sim::components::ComponentId;
use crate::sim::handle::{Handle, HandleCtx};
use crate::sim::memory::GlobalMemory;
use crate::sim::memory::write_log::Writer;
use crate::sim::packet::{AccessSize, HitLevel, MemOp, MemRespData, MesiState, Packet, WriteData};
use crate::soc::devices::Device;
use crate::soc::exit::ExitSignal;
use crate::soc::terminal::SharedTerminal;
use crate::system::process::errno::Errno;
use crate::system::process::file_calls::{self, FileCalls, GuestMemory, PhysicalMemory};
use crate::system::process::files::{AT_FDCWD, Fd, FileTable, PATH_MAX};
use crate::system::process::syscall::{
    SYS_CHDIR, SYS_CLOSE, SYS_EXIT, SYS_FACCESSAT, SYS_FCNTL, SYS_FSTAT, SYS_FTRUNCATE, SYS_GETCWD,
    SYS_LSEEK, SYS_MKDIRAT, SYS_NEWFSTATAT, SYS_OPENAT, SYS_PREAD64, SYS_PWRITE64, SYS_READ,
    SYS_READLINKAT, SYS_RENAMEAT, SYS_UNLINKAT, SYS_WRITE,
};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

const DEVICE_SHIFT: u64 = 56;
const COMMAND_SHIFT: u64 = 48;
const PAYLOAD_MASK: u64 = (1 << COMMAND_SHIFT) - 1;
const SYSCALL_DEVICE: u64 = 0;
const SYSCALL_COMMAND: u64 = 0;
const BLOCK_WORDS: usize = 8;
const FROMHOST_OFFSET: u64 = 8;

/// fesvr's own calls, beyond Linux's.
const SYS_LSTAT: u64 = 1039;
const SYS_GETMAINVARS: u64 = 2011;

/// HTIF device: serves `tohost` and `fromhost`.
#[derive(Debug)]
pub struct Htif {
    base_addr: u64,
    exit_signal: ExitSignal,
    /// What the target reads at `fromhost` until it clears it.
    fromhost: u64,
    /// The host's descriptors; 0, 1 and 2 are the simulator's console.
    files: FileTable,
    /// What `getmainvars` hands the target.
    args: Vec<String>,
}

impl Htif {
    /// A device at `base_addr`, the `tohost` symbol, sharing `exit_signal`
    /// with the run and answering `getmainvars` with `args`.
    #[must_use]
    pub fn new(
        base_addr: u64,
        exit_signal: ExitSignal,
        args: Vec<String>,
        terminal: SharedTerminal,
    ) -> Self {
        Self { base_addr, exit_signal, fromhost: 0, files: FileTable::new(terminal), args }
    }

    fn handle_tohost(&mut self, val: u64, memory: &mut GlobalMemory) {
        if val == 0 {
            return;
        }
        let device = val >> DEVICE_SHIFT;
        let command = (val >> COMMAND_SHIFT) & 0xff;
        let payload = val & PAYLOAD_MASK;
        if (device, command) != (SYSCALL_DEVICE, SYSCALL_COMMAND) {
            tracing::warn!(target: "rvsim::htif", device, command, payload, "unhandled HTIF device command");
            self.fromhost = (val >> COMMAND_SHIFT << COMMAND_SHIFT) | 1;
            return;
        }
        if payload & 1 != 0 {
            let code = payload >> 1;
            if code != 0 {
                tracing::warn!(target: "rvsim::htif", test = code, tohost = val, "test failed");
            }
            self.exit_signal.signal(code);
            return;
        }
        self.serve_syscall(payload, memory);
        self.fromhost = (val >> COMMAND_SHIFT << COMMAND_SHIFT) | 1;
    }

    /// Performs the syscall whose block is at `block`, as fesvr does, and
    /// leaves the result in the block's first word.
    fn serve_syscall(&mut self, block: u64, memory: &mut GlobalMemory) {
        let Some(bytes) = memory.read_bytes(PhysAddr::new(block), BLOCK_WORDS * 8) else {
            tracing::warn!(target: "rvsim::htif", block, "syscall block outside RAM");
            return;
        };
        let words: Vec<u64> =
            bytes.as_chunks::<8>().0.iter().map(|word| u64::from_le_bytes(*word)).collect();
        let number = words[0];
        let a: [u64; 6] = std::array::from_fn(|i| words[i + 1]);
        let result = self.dispatch(number, a, memory).unwrap_or_else(Errno::return_value);
        tracing::trace!(target: "rvsim::htif", number, a0 = a[0], a1 = a[1], a2 = a[2], result, "syscall");
        memory.write_bytes(Writer::External, PhysAddr::new(block), &result.to_le_bytes());
    }

    fn dispatch(
        &mut self,
        number: u64,
        a: [u64; 6],
        memory: &mut GlobalMemory,
    ) -> Result<u64, Errno> {
        let at = |i: usize| PhysAddr::new(a[i]);
        let fd = Fd::from_arg(a[0]);
        let mut calls = FileCalls { files: &mut self.files, memory: PhysicalMemory(memory) };
        match number {
            SYS_EXIT => {
                self.exit_signal.signal(a[0]);
                Ok(0)
            }
            SYS_WRITE => calls.write(fd, at(1), a[2]),
            SYS_READ => calls.read(fd, at(1), a[2]),
            SYS_PREAD64 => calls.pread(fd, at(1), a[2], a[3]),
            SYS_PWRITE64 => calls.pwrite(fd, at(1), a[2], a[3]),
            SYS_OPENAT => {
                let path = fesvr_path(&mut calls, a[1], a[2])?;
                calls.files.open(path, a[3], a[4]).map(Fd::return_value)
            }
            SYS_CLOSE => calls.files.close(fd).map(|()| 0),
            SYS_LSEEK => calls.files.seek(fd, a[1] as i64, a[2]),
            SYS_FSTAT => calls.fstat(fd, at(1)),
            SYS_NEWFSTATAT => {
                let stat = file_calls::stat(&fesvr_path(&mut calls, a[1], a[2])?)?;
                calls.write_stat(stat, at(3))
            }
            SYS_LSTAT => {
                let stat = file_calls::lstat(&fesvr_path(&mut calls, a[0], a[1])?)?;
                calls.write_stat(stat, at(2))
            }
            SYS_FTRUNCATE => calls.files.truncate(fd, a[1]).map(|()| 0),
            SYS_FACCESSAT => file_calls::access(&fesvr_path(&mut calls, a[1], a[2])?),
            SYS_MKDIRAT => file_calls::make_dir(&fesvr_path(&mut calls, a[1], a[2])?),
            SYS_UNLINKAT => file_calls::unlink(&fesvr_path(&mut calls, a[1], a[2])?, a[3]),
            SYS_RENAMEAT => {
                let from = fesvr_path(&mut calls, a[1], a[2])?;
                file_calls::rename(&from, &fesvr_path(&mut calls, a[4], a[5])?)
            }
            SYS_READLINKAT => {
                let target = file_calls::read_link(&fesvr_path(&mut calls, a[1], a[2])?)?;
                calls.write_link_target(&target, at(3), a[4])
            }
            SYS_GETCWD => calls.getcwd(at(0), a[1]),
            SYS_CHDIR => {
                let path = cstr(&calls.memory, at(0))?;
                std::env::set_current_dir(OsStr::from_bytes(&path))?;
                Ok(0)
            }
            SYS_FCNTL => Ok(0),
            SYS_GETMAINVARS => getmainvars(&mut calls.memory, &self.args, at(0), a[1]),
            _ => {
                tracing::warn!(target: "rvsim::htif", number, "unimplemented HTIF syscall; returning ENOSYS");
                Err(Errno::ENOSYS)
            }
        }
    }
}

/// A path the target passed as an address and a length that includes its
/// NUL, as fesvr's callers pass them.
fn fesvr_path(
    calls: &mut FileCalls<'_, PhysicalMemory<'_>>,
    addr: u64,
    len: u64,
) -> Result<PathBuf, Errno> {
    let mut bytes = calls.memory.read(PhysAddr::new(addr), file_calls::buffer_len(len)?)?;
    if let Some(nul) = bytes.iter().position(|&b| b == 0) {
        bytes.truncate(nul);
    }
    calls.files.resolve(AT_FDCWD, &bytes)
}

/// The NUL-terminated string at `addr`.
fn cstr(memory: &PhysicalMemory<'_>, addr: PhysAddr) -> Result<Vec<u8>, Errno> {
    let mut out = Vec::new();
    let mut cursor = addr;
    loop {
        let [byte] = memory.read(cursor, 1)?[..] else { return Err(Errno::EFAULT) };
        if byte == 0 || out.len() >= PATH_MAX {
            return Ok(out);
        }
        out.push(byte);
        cursor = PhysAddr::new(cursor.val() + 1);
    }
}

/// `argc`, the `argv` pointers, a NULL, an empty `envp`, then the strings,
/// written at `pbuf` when they fit in `limit` bytes.
fn getmainvars(
    memory: &mut PhysicalMemory<'_>,
    args: &[String],
    pbuf: PhysAddr,
    limit: u64,
) -> Result<u64, Errno> {
    let mut words: Vec<u64> = vec![args.len() as u64];
    let mut size = (args.len() as u64 + 3) * 8;
    for arg in args {
        words.push(pbuf.val() + size);
        size += arg.len() as u64 + 1;
    }
    words.push(0);
    words.push(0);
    let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    for arg in args {
        bytes.extend_from_slice(arg.as_bytes());
        bytes.push(0);
    }
    if bytes.len() as u64 > limit {
        return Err(Errno::ENOMEM);
    }
    memory.write(pbuf, &bytes)?;
    Ok(0)
}

impl Handle for Htif {
    fn handle(&mut self, packet: Packet, source: ComponentId, ctx: &mut HandleCtx<'_>) {
        if let Packet::MemReq { req_id, paddr, size, op, .. } = packet {
            let offset = paddr.val().saturating_sub(self.base_addr);
            let word = matches!(size, AccessSize::B4 | AccessSize::B8);
            // Only word- or doubleword-sized accesses touch the protocol;
            // spike's HTIF ignores sub-word partial writes too.
            let data = match (offset, op, word) {
                (0, MemOp::Write { data: WriteData::Small(val), .. }, true) => {
                    self.handle_tohost(val, ctx.memory);
                    0
                }
                (FROMHOST_OFFSET, MemOp::Write { data: WriteData::Small(val), .. }, true) => {
                    self.fromhost = val;
                    0
                }
                (FROMHOST_OFFSET, MemOp::Read | MemOp::ReadOwn | MemOp::Atomic { .. }, _) => {
                    self.fromhost
                }
                _ => 0,
            };
            ctx.scheduler.schedule(
                ctx.cycle + ctx.config.system.device_access_cycles(self.name()),
                source,
                ctx.self_id,
                Packet::MemResp {
                    req_id,
                    line_addr: LineAddr::from_phys(paddr, 64),
                    data: MemRespData::Small(data),
                    hit_level: HitLevel::Mmio,
                    state: MesiState::Exclusive,
                },
            );
        }
    }
}

/// The device's state in a checkpoint. The host files the program had
/// open are counted so a restore can refuse what it cannot give back.
#[derive(Serialize, Deserialize)]
struct HtifState {
    fromhost: u64,
    host_files_open: usize,
}

impl HtifState {
    fn parse(state: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(state.clone()).map_err(|error| format!("HTIF state: {error}"))
    }
}

impl Device for Htif {
    fn name(&self) -> &'static str {
        "HTIF"
    }

    fn checkpoint(&self) -> Option<serde_json::Value> {
        let state =
            HtifState { fromhost: self.fromhost, host_files_open: self.files.host_files_open() };
        serde_json::to_value(state).ok()
    }

    fn check_restore(&self, state: &serde_json::Value) -> Result<(), String> {
        let state = HtifState::parse(state)?;
        if state.host_files_open > 0 {
            return Err(format!(
                "HTIF: the program had {} host files open at the checkpoint, which a restored run cannot get back",
                state.host_files_open
            ));
        }
        Ok(())
    }

    fn restore(&mut self, state: &serde_json::Value) -> Result<(), String> {
        self.check_restore(state)?;
        self.fromhost = HtifState::parse(state)?.fromhost;
        Ok(())
    }

    fn address_range(&self) -> (u64, u64) {
        (self.base_addr, 16)
    }

    fn quiet_ticks(&self) -> Option<u64> {
        None
    }
}
