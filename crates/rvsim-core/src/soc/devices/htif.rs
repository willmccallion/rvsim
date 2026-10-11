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
use crate::system::process::files::{FileTable, Stat, errno, io_errno};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

const DEVICE_SHIFT: u64 = 56;
const COMMAND_SHIFT: u64 = 48;
const PAYLOAD_MASK: u64 = (1 << COMMAND_SHIFT) - 1;
const SYSCALL_DEVICE: u64 = 0;
const SYSCALL_COMMAND: u64 = 0;
const BLOCK_WORDS: usize = 8;
const FROMHOST_OFFSET: u64 = 8;

const SYS_GETCWD: u64 = 17;
const SYS_FCNTL: u64 = 25;
const SYS_MKDIRAT: u64 = 34;
const SYS_UNLINKAT: u64 = 35;
const SYS_RENAMEAT: u64 = 38;
const SYS_FTRUNCATE: u64 = 46;
const SYS_FACCESSAT: u64 = 48;
const SYS_CHDIR: u64 = 49;
const SYS_OPENAT: u64 = 56;
const SYS_CLOSE: u64 = 57;
const SYS_LSEEK: u64 = 62;
const SYS_READ: u64 = 63;
const SYS_WRITE: u64 = 64;
const SYS_PREAD: u64 = 67;
const SYS_PWRITE: u64 = 68;
const SYS_READLINKAT: u64 = 78;
const SYS_FSTATAT: u64 = 79;
const SYS_FSTAT: u64 = 80;
const SYS_EXIT: u64 = 93;
const SYS_LSTAT: u64 = 1039;
const SYS_GETMAINVARS: u64 = 2011;

/// HTIF device: serves `tohost` and `fromhost`.
#[derive(Debug)]
pub struct Htif {
    base_addr: u64,
    exit_signal: Arc<AtomicU64>,
    /// What the target reads at `fromhost` until it clears it.
    fromhost: u64,
    /// The host's descriptors; stderr goes to stdout, as fesvr maps it.
    files: FileTable,
    /// What `getmainvars` hands the target.
    args: Vec<String>,
}

impl Htif {
    /// A device at `base_addr`, the `tohost` symbol, sharing `exit_signal`
    /// with the run and answering `getmainvars` with `args`.
    #[must_use]
    pub fn new(base_addr: u64, exit_signal: Arc<AtomicU64>, args: Vec<String>) -> Self {
        Self {
            base_addr,
            exit_signal,
            fromhost: 0,
            files: FileTable::with_stderr_on_stdout(),
            args,
        }
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
            self.exit_signal.store(code, Ordering::Relaxed);
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
        let result = match self.dispatch(number, a, memory) {
            Ok(value) => value,
            Err(negated_errno) => negated_errno as u64,
        };
        tracing::trace!(target: "rvsim::htif", number, a0 = a[0], a1 = a[1], a2 = a[2], result, "syscall");
        memory.write_bytes(Writer::External, PhysAddr::new(block), &result.to_le_bytes());
    }

    fn dispatch(
        &mut self,
        number: u64,
        a: [u64; 6],
        memory: &mut GlobalMemory,
    ) -> Result<u64, i64> {
        let mut host = Host { files: &mut self.files, memory };
        match number {
            SYS_EXIT => {
                self.exit_signal.store(a[0], Ordering::Relaxed);
                Ok(0)
            }
            SYS_WRITE => host.write(a[0] as i64, a[1], a[2]),
            SYS_READ => host.read(a[0] as i64, a[1], a[2]),
            SYS_PREAD => host.pread(a[0] as i64, a[1], a[2], a[3]),
            SYS_PWRITE => host.pwrite(a[0] as i64, a[1], a[2], a[3]),
            SYS_OPENAT => {
                let path = host.path(a[1], a[2])?;
                host.files.open(path, a[3], a[4]).map(|fd| fd as u64)
            }
            SYS_CLOSE => host.files.close(a[0] as i64).map(|()| 0),
            SYS_LSEEK => host.files.seek(a[0] as i64, a[1] as i64, a[2]),
            SYS_FSTAT => {
                let stat = host.files.stat(a[0] as i64)?;
                {
                    host.write_bytes(a[1], &stat.to_bytes());
                    Ok(0)
                }
            }
            SYS_FSTATAT => {
                let path = host.path(a[1], a[2])?;
                let stat = std::fs::metadata(&path)
                    .map(|m| Stat::from_metadata(&m))
                    .map_err(|e| io_errno(&e))?;
                {
                    host.write_bytes(a[3], &stat.to_bytes());
                    Ok(0)
                }
            }
            SYS_LSTAT => {
                let path = host.path(a[0], a[1])?;
                let stat = std::fs::symlink_metadata(&path)
                    .map(|m| Stat::from_metadata(&m))
                    .map_err(|e| io_errno(&e))?;
                {
                    host.write_bytes(a[2], &stat.to_bytes());
                    Ok(0)
                }
            }
            SYS_FTRUNCATE => host.files.truncate(a[0] as i64, a[1]).map(|()| 0),
            SYS_FACCESSAT => {
                let path = host.path(a[1], a[2])?;
                std::fs::metadata(&path).map(drop).map_err(|e| io_errno(&e))?;
                Ok(0)
            }
            SYS_MKDIRAT => {
                let path = host.path(a[1], a[2])?;
                std::fs::create_dir(&path).map_err(|e| io_errno(&e))?;
                Ok(0)
            }
            SYS_UNLINKAT => {
                let path = host.path(a[1], a[2])?;
                let removed = if a[3] & 0x200 != 0 {
                    std::fs::remove_dir(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                removed.map_err(|e| io_errno(&e))?;
                Ok(0)
            }
            SYS_RENAMEAT => {
                let from = host.path(a[1], a[2])?;
                let to = host.path(a[4], a[5])?;
                std::fs::rename(&from, &to).map_err(|e| io_errno(&e))?;
                Ok(0)
            }
            SYS_READLINKAT => {
                let path = host.path(a[1], a[2])?;
                let target = std::fs::read_link(&path)
                    .map(|p| p.to_string_lossy().into_owned().into_bytes())
                    .map_err(|e| io_errno(&e))?;
                let len = target.len().min(usize::try_from(a[4]).map_err(|_| -errno::EINVAL)?);
                host.write_bytes(a[3], &target[..len]);
                Ok(len as u64)
            }
            SYS_GETCWD => {
                let mut cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned().into_bytes())
                    .map_err(|e| io_errno(&e))?;
                cwd.push(0);
                if cwd.len() as u64 > a[1] {
                    return Err(-errno::ENOMEM);
                }
                host.write_bytes(a[0], &cwd);
                Ok(cwd.len() as u64)
            }
            SYS_CHDIR => {
                let path = host.cstr(a[0])?;
                std::env::set_current_dir(String::from_utf8_lossy(&path).as_ref())
                    .map_err(|e| io_errno(&e))?;
                Ok(0)
            }
            SYS_FCNTL => Ok(0),
            SYS_GETMAINVARS => host.getmainvars(&self.args, a[0], a[1]),
            _ => {
                tracing::warn!(target: "rvsim::htif", number, "unimplemented HTIF syscall; returning ENOSYS");
                Err(-errno::ENOSYS)
            }
        }
    }
}

/// The host's side of one syscall: its files and the target's memory.
struct Host<'a> {
    files: &'a mut FileTable,
    memory: &'a mut GlobalMemory,
}

impl Host<'_> {
    fn read_bytes(&self, addr: u64, len: u64) -> Result<Vec<u8>, i64> {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        self.memory.read_bytes(PhysAddr::new(addr), len).map(Vec::from).ok_or(-errno::EFAULT)
    }

    fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.memory.write_bytes(Writer::External, PhysAddr::new(addr), bytes);
        }
    }

    /// A path the target passed as an address and a length that includes
    /// its NUL, as fesvr's callers pass them.
    fn path(&mut self, addr: u64, len: u64) -> Result<std::path::PathBuf, i64> {
        let mut bytes = self.read_bytes(addr, len)?;
        if let Some(nul) = bytes.iter().position(|&b| b == 0) {
            bytes.truncate(nul);
        }
        self.files.resolve(crate::system::process::files::AT_FDCWD, &bytes)
    }

    /// A NUL-terminated string at `addr`.
    fn cstr(&self, addr: u64) -> Result<Vec<u8>, i64> {
        let mut out = Vec::new();
        let mut cursor = addr;
        loop {
            let byte = self.memory.read(PhysAddr::new(cursor), 1).ok_or(-errno::EFAULT)? as u8;
            if byte == 0 || out.len() >= 4096 {
                return Ok(out);
            }
            out.push(byte);
            cursor += 1;
        }
    }

    fn write(&mut self, fd: i64, buf: u64, len: u64) -> Result<u64, i64> {
        let bytes = self.read_bytes(buf, len)?;
        self.files.write(fd, &bytes).map(|n| n as u64)
    }

    fn read(&mut self, fd: i64, buf: u64, len: u64) -> Result<u64, i64> {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        let bytes = self.files.read(fd, len)?;
        self.write_bytes(buf, &bytes);
        Ok(bytes.len() as u64)
    }

    fn pread(&mut self, fd: i64, buf: u64, len: u64, offset: u64) -> Result<u64, i64> {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        let bytes = self.files.pread(fd, len, offset)?;
        self.write_bytes(buf, &bytes);
        Ok(bytes.len() as u64)
    }

    fn pwrite(&mut self, fd: i64, buf: u64, len: u64, offset: u64) -> Result<u64, i64> {
        let bytes = self.read_bytes(buf, len)?;
        self.files.pwrite(fd, &bytes, offset).map(|n| n as u64)
    }

    /// `argc`, the `argv` pointers, a NULL, an empty `envp`, then the
    /// strings, written at `pbuf` when they fit in `limit` bytes.
    fn getmainvars(&mut self, args: &[String], pbuf: u64, limit: u64) -> Result<u64, i64> {
        let mut words: Vec<u64> = vec![args.len() as u64];
        let mut size = (args.len() as u64 + 3) * 8;
        for arg in args {
            words.push(pbuf + size);
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
            return Err(-errno::ENOMEM);
        }
        self.write_bytes(pbuf, &bytes);
        Ok(0)
    }
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
