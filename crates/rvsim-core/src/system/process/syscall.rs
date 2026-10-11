//! The Linux syscalls SE mode serves when a user program's `ecall`
//! retires. Numbers are riscv64's (`asm-generic/unistd.h`); results are
//! the value or the negated `errno`, as the kernel returns them.

use super::files::{Stat, errno, io_errno};
use super::{PAGE_SIZE, Permissions, Process, SpaceError, page_ceil};
use crate::arch::translation::SfenceVmaInfo;
use crate::isa::reg::{self, RegIdx};
use crate::sim::memory::GlobalMemory;
use crate::uarch::ctx::CoreCtx;

/// What retiring the `ecall` leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Carry on at the instruction after the `ecall`, with the result in `a0`.
    Continue {
        /// The syscall's return value, or its negated `errno`.
        a0: u64,
    },
    /// The program ended with this exit code.
    Exit(u64),
}

const SYS_GETCWD: u64 = 17;
const SYS_FCNTL: u64 = 25;
const SYS_IOCTL: u64 = 29;
const SYS_MKDIRAT: u64 = 34;
const SYS_UNLINKAT: u64 = 35;
const SYS_RENAMEAT: u64 = 38;
const SYS_FACCESSAT: u64 = 48;
const SYS_OPENAT: u64 = 56;
const SYS_CLOSE: u64 = 57;
const SYS_LSEEK: u64 = 62;
const SYS_READ: u64 = 63;
const SYS_WRITE: u64 = 64;
const SYS_READV: u64 = 65;
const SYS_WRITEV: u64 = 66;
const SYS_PREAD64: u64 = 67;
const SYS_PWRITE64: u64 = 68;
const SYS_READLINKAT: u64 = 78;
const SYS_NEWFSTATAT: u64 = 79;
const SYS_FSTAT: u64 = 80;
const SYS_EXIT: u64 = 93;
const SYS_EXIT_GROUP: u64 = 94;
const SYS_SET_TID_ADDRESS: u64 = 96;
const SYS_FUTEX: u64 = 98;
const SYS_SET_ROBUST_LIST: u64 = 99;
const SYS_NANOSLEEP: u64 = 101;
const SYS_CLOCK_GETTIME: u64 = 113;
const SYS_CLOCK_GETRES: u64 = 114;
const SYS_CLOCK_NANOSLEEP: u64 = 115;
const SYS_SCHED_YIELD: u64 = 124;
const SYS_KILL: u64 = 129;
const SYS_TKILL: u64 = 130;
const SYS_TGKILL: u64 = 131;
const SYS_RT_SIGACTION: u64 = 134;
const SYS_RT_SIGPROCMASK: u64 = 135;
const SYS_TIMES: u64 = 153;
const SYS_UNAME: u64 = 160;
const SYS_GETRUSAGE: u64 = 165;
const SYS_GETTIMEOFDAY: u64 = 169;
const SYS_GETPID: u64 = 172;
const SYS_GETPPID: u64 = 173;
const SYS_GETUID: u64 = 174;
const SYS_GETEUID: u64 = 175;
const SYS_GETGID: u64 = 176;
const SYS_GETEGID: u64 = 177;
const SYS_GETTID: u64 = 178;
const SYS_SYSINFO: u64 = 179;
const SYS_BRK: u64 = 214;
const SYS_MUNMAP: u64 = 215;
const SYS_MREMAP: u64 = 216;
const SYS_CLONE: u64 = 220;
const SYS_EXECVE: u64 = 221;
const SYS_MMAP: u64 = 222;
const SYS_MPROTECT: u64 = 226;
const SYS_MADVISE: u64 = 233;
const SYS_RISCV_HWPROBE: u64 = 258;
const SYS_RISCV_FLUSH_ICACHE: u64 = 259;
const SYS_PRLIMIT64: u64 = 261;
const SYS_RENAMEAT2: u64 = 276;
const SYS_GETRANDOM: u64 = 278;
const SYS_STATX: u64 = 291;
const SYS_RSEQ: u64 = 293;
const SYS_CLONE3: u64 = 435;
const SYS_FACCESSAT2: u64 = 439;

const MAP_FIXED: u64 = 0x10;
const MAP_ANONYMOUS: u64 = 0x20;
const MREMAP_MAYMOVE: u64 = 1;
const AT_EMPTY_PATH: u64 = 0x1000;
const AT_REMOVEDIR: u64 = 0x200;
const TCGETS: u32 = 0x5401;
/// `TCGETS2`, which glibc 2.42's `tcgetattr` (and so `isatty`) uses.
const TCGETS2: u32 = 0x802c_542a;
const TIOCGWINSZ: u32 = 0x5413;
const RLIMIT_STACK: u64 = 3;
const RLIMIT_NOFILE: u64 = 7;
const RLIM_INFINITY: u64 = u64::MAX;
const FUTEX_WAIT: u64 = 0;
const FUTEX_CMD_MASK: u64 = 0x7f;
const EAGAIN: i64 = 11;
/// The epoch `CLOCK_REALTIME` starts from: 2023-11-14, so dates are sane
/// and every run sees the same clock.
const REALTIME_EPOCH_SECONDS: u64 = 1_700_000_000;
const PATH_MAX: usize = 4096;
const PID: u64 = 1;
const TID: u64 = 1;

/// Serves the `ecall` that retired at `pc` on `state`'s hart: performs
/// the syscall in `a7` with the arguments in `a0` to `a5`, leaves the
/// result in `a0`, and says whether the program goes on.
pub fn serve_ecall(state: &mut CoreCtx<'_>, pc: u64) -> Outcome {
    let number = state.hart.regs.read(reg::REG_A7);
    let args: [u64; 6] = std::array::from_fn(|i| state.hart.regs.read(RegIdx::new(10 + i as u8)));
    let cycle = state.uncore.cycle;
    let ram_bytes = state.uncore.config.memory.ram_size as u64;
    let uncore = &mut *state.uncore;
    let Some(process) = uncore.process.as_mut() else {
        return Outcome::Exit(1);
    };
    *process.syscall_counts.entry(number).or_insert(0) += 1;
    let mut sys = Sys { process, memory: &mut uncore.memory, cycle, ram_bytes };
    let result = sys.dispatch(number, args);
    tracing::trace!(target: "rvsim::syscall", pc = %crate::common::trace::Hex(pc), number, name = name_of(number),
        a0 = %crate::common::trace::Hex(args[0]), a1 = %crate::common::trace::Hex(args[1]),
        a2 = %crate::common::trace::Hex(args[2]), a3 = %crate::common::trace::Hex(args[3]),
        result = ?result, "syscall");
    match result {
        Ok(Reply::Value(value)) => Outcome::Continue { a0: value as u64 },
        Ok(Reply::Exit(code)) => Outcome::Exit(code),
        Err(negated_errno) => Outcome::Continue { a0: negated_errno as u64 },
    }
}

/// What a handler produces.
#[derive(Debug, Clone, Copy)]
enum Reply {
    Value(i64),
    Exit(u64),
}

type Result = std::result::Result<Reply, i64>;

/// Everything a handler touches.
struct Sys<'a> {
    process: &'a mut Process,
    memory: &'a mut GlobalMemory,
    cycle: u64,
    ram_bytes: u64,
}

#[allow(clippy::unnecessary_wraps, reason = "the handlers' common result type")]
const fn ok(value: u64) -> Result {
    Ok(Reply::Value(value as i64))
}

const fn fault(error: SpaceError) -> i64 {
    match error {
        SpaceError::OutOfFrames => -errno::ENOMEM,
        SpaceError::OutOfRange(_) | SpaceError::Unmapped(_) => -errno::EFAULT,
    }
}

impl Sys<'_> {
    fn dispatch(&mut self, number: u64, a: [u64; 6]) -> Result {
        match number {
            SYS_EXIT | SYS_EXIT_GROUP => Ok(Reply::Exit(a[0] & 0xff)),
            SYS_KILL | SYS_TKILL | SYS_TGKILL => {
                let signal = if number == SYS_KILL { a[1] } else { a[2] };
                Ok(Reply::Exit(128 + (signal & 0x7f)))
            }
            SYS_WRITE => self.write(a[0] as i64, a[1], a[2]),
            SYS_READ => self.read(a[0] as i64, a[1], a[2]),
            SYS_WRITEV => self.writev(a[0] as i64, a[1], a[2]),
            SYS_READV => self.readv(a[0] as i64, a[1], a[2]),
            SYS_PREAD64 => self.pread(a[0] as i64, a[1], a[2], a[3]),
            SYS_PWRITE64 => self.pwrite(a[0] as i64, a[1], a[2], a[3]),
            SYS_OPENAT => self.openat(a[0] as i64, a[1], a[2], a[3]),
            SYS_CLOSE => self.process.files.close(a[0] as i64).map(|()| Reply::Value(0)),
            SYS_LSEEK => self.process.files.seek(a[0] as i64, a[1] as i64, a[2]).and_then(ok),
            SYS_FSTAT => self.fstat(a[0] as i64, a[1]),
            SYS_NEWFSTATAT => self.fstatat(a[0] as i64, a[1], a[2], a[3]),
            SYS_FACCESSAT | SYS_FACCESSAT2 => self.faccessat(a[0] as i64, a[1]),
            SYS_READLINKAT => self.readlinkat(a[0] as i64, a[1], a[2], a[3]),
            SYS_MKDIRAT => self.mkdirat(a[0] as i64, a[1]),
            SYS_UNLINKAT => self.unlinkat(a[0] as i64, a[1], a[2]),
            SYS_RENAMEAT | SYS_RENAMEAT2 => self.renameat(a[0] as i64, a[1], a[2] as i64, a[3]),
            SYS_GETCWD => self.getcwd(a[0], a[1]),
            SYS_IOCTL => self.ioctl(a[0] as i64, a[1], a[2]),
            SYS_BRK => {
                let brk = self.process.set_brk(self.memory, a[0]);
                ok(brk)
            }
            SYS_MMAP => self.mmap(a[0], a[1], a[2], a[3], a[4] as i64, a[5]),
            SYS_MUNMAP => {
                self.process.unmap(self.memory, a[0], a[1]);
                ok(0)
            }
            SYS_MREMAP => self.mremap(a[0], a[1], a[2], a[3]),
            SYS_MPROTECT => self.mprotect(a[0], a[1], a[2]),
            SYS_FCNTL
            | SYS_GETPPID
            | SYS_GETUID
            | SYS_GETEUID
            | SYS_GETGID
            | SYS_GETEGID
            | SYS_MADVISE
            | SYS_RISCV_FLUSH_ICACHE
            | SYS_SCHED_YIELD
            | SYS_NANOSLEEP
            | SYS_CLOCK_NANOSLEEP
            | SYS_SET_ROBUST_LIST => ok(0),
            SYS_SET_TID_ADDRESS => {
                self.process.clear_child_tid = a[0];
                ok(TID)
            }
            SYS_FUTEX => {
                // One thread: a wait would never be woken, so it reports the
                // value changed; a wake wakes nobody.
                if a[1] & FUTEX_CMD_MASK == FUTEX_WAIT { Err(-EAGAIN) } else { ok(0) }
            }
            SYS_RT_SIGACTION => self.zero_if_given(a[2], 24),
            SYS_RT_SIGPROCMASK => self.zero_if_given(a[2], a[3] as usize),
            SYS_CLOCK_GETTIME => self.clock_gettime(a[0], a[1]),
            SYS_CLOCK_GETRES => self.write_timespec(a[1], 0, 1),
            SYS_GETTIMEOFDAY => self.gettimeofday(a[0]),
            SYS_TIMES => self.times(a[0]),
            SYS_GETRUSAGE => self.getrusage(a[1]),
            SYS_UNAME => self.uname(a[0]),
            SYS_SYSINFO => self.sysinfo(a[0]),
            SYS_GETPID | SYS_GETTID => ok(PID),
            SYS_PRLIMIT64 => self.prlimit(a[1], a[3]),
            SYS_GETRANDOM => self.getrandom(a[0], a[1]),
            // What a kernel without them answers; glibc falls back.
            SYS_CLONE | SYS_CLONE3 | SYS_EXECVE | SYS_STATX | SYS_RSEQ | SYS_RISCV_HWPROBE => {
                Err(-errno::ENOSYS)
            }
            _ => {
                tracing::warn!(target: "rvsim::syscall", number, "unimplemented syscall; returning ENOSYS");
                Err(-errno::ENOSYS)
            }
        }
    }

    fn read_user(&self, vaddr: u64, len: u64) -> std::result::Result<Vec<u8>, i64> {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        self.process.space.read(self.memory, vaddr, len).map_err(fault)
    }

    fn write_user(&mut self, vaddr: u64, bytes: &[u8]) -> std::result::Result<(), i64> {
        self.process.space.write(self.memory, vaddr, bytes).map_err(fault)
    }

    fn read_path(&self, vaddr: u64) -> std::result::Result<Vec<u8>, i64> {
        self.process.space.read_cstr(self.memory, vaddr, PATH_MAX).map_err(fault)
    }

    fn resolve(&mut self, dirfd: i64, path: u64) -> std::result::Result<std::path::PathBuf, i64> {
        let path = self.read_path(path)?;
        self.process.files.resolve(dirfd, &path)
    }

    fn write(&mut self, fd: i64, buf: u64, len: u64) -> Result {
        let bytes = self.read_user(buf, len)?;
        self.process.files.write(fd, &bytes).and_then(|n| ok(n as u64))
    }

    fn read(&mut self, fd: i64, buf: u64, len: u64) -> Result {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        let bytes = self.process.files.read(fd, len)?;
        self.write_user(buf, &bytes)?;
        ok(bytes.len() as u64)
    }

    fn pread(&mut self, fd: i64, buf: u64, len: u64, offset: u64) -> Result {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        let bytes = self.process.files.pread(fd, len, offset)?;
        self.write_user(buf, &bytes)?;
        ok(bytes.len() as u64)
    }

    fn pwrite(&mut self, fd: i64, buf: u64, len: u64, offset: u64) -> Result {
        let bytes = self.read_user(buf, len)?;
        self.process.files.pwrite(fd, &bytes, offset).and_then(|n| ok(n as u64))
    }

    /// The `(base, len)` pairs of an `iovec` array.
    fn iovecs(&self, iov: u64, count: u64) -> std::result::Result<Vec<(u64, u64)>, i64> {
        let count = usize::try_from(count).map_err(|_| -errno::EINVAL)?;
        let bytes = self.read_user(iov, (count * 16) as u64)?;
        Ok(bytes
            .as_chunks::<16>()
            .0
            .iter()
            .map(|pair| {
                let mut base = [0u8; 8];
                let mut len = [0u8; 8];
                base.copy_from_slice(&pair[..8]);
                len.copy_from_slice(&pair[8..]);
                (u64::from_le_bytes(base), u64::from_le_bytes(len))
            })
            .collect())
    }

    fn writev(&mut self, fd: i64, iov: u64, count: u64) -> Result {
        let mut total = 0u64;
        for (base, len) in self.iovecs(iov, count)? {
            let bytes = self.read_user(base, len)?;
            total += self.process.files.write(fd, &bytes)? as u64;
        }
        ok(total)
    }

    fn readv(&mut self, fd: i64, iov: u64, count: u64) -> Result {
        let mut total = 0u64;
        for (base, len) in self.iovecs(iov, count)? {
            let want = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
            let bytes = self.process.files.read(fd, want)?;
            self.write_user(base, &bytes)?;
            total += bytes.len() as u64;
            if bytes.len() < want {
                break;
            }
        }
        ok(total)
    }

    fn openat(&mut self, dirfd: i64, path: u64, flags: u64, mode: u64) -> Result {
        let path = self.resolve(dirfd, path)?;
        self.process.files.open(path, flags, mode).and_then(ok_i64)
    }

    fn fstat(&mut self, fd: i64, buf: u64) -> Result {
        let stat = self.process.files.stat(fd)?;
        self.write_user(buf, &stat.to_bytes())?;
        ok(0)
    }

    fn fstatat(&mut self, dirfd: i64, path: u64, buf: u64, flags: u64) -> Result {
        let path_bytes = self.read_path(path)?;
        let stat = if path_bytes.is_empty() && flags & AT_EMPTY_PATH != 0 {
            self.process.files.stat(dirfd)?
        } else {
            let host = self.process.files.resolve(dirfd, &path_bytes)?;
            std::fs::metadata(&host)
                .map(|meta| Stat::from_metadata(&meta))
                .map_err(|e| io_errno(&e))?
        };
        self.write_user(buf, &stat.to_bytes())?;
        ok(0)
    }

    fn faccessat(&mut self, dirfd: i64, path: u64) -> Result {
        let host = self.resolve(dirfd, path)?;
        std::fs::metadata(&host).map(drop).map_err(|e| io_errno(&e))?;
        ok(0)
    }

    fn readlinkat(&mut self, dirfd: i64, path: u64, buf: u64, size: u64) -> Result {
        let path_bytes = self.read_path(path)?;
        let target: Vec<u8> = if path_bytes == b"/proc/self/exe" {
            std::env::current_exe()
                .map(|p| p.to_string_lossy().into_owned().into_bytes())
                .unwrap_or_default()
        } else {
            let host = self.process.files.resolve(dirfd, &path_bytes)?;
            std::fs::read_link(&host)
                .map(|p| p.to_string_lossy().into_owned().into_bytes())
                .map_err(|e| io_errno(&e))?
        };
        let len = target.len().min(usize::try_from(size).map_err(|_| -errno::EINVAL)?);
        self.write_user(buf, &target[..len])?;
        ok(len as u64)
    }

    fn mkdirat(&mut self, dirfd: i64, path: u64) -> Result {
        let host = self.resolve(dirfd, path)?;
        std::fs::create_dir(&host).map_err(|e| io_errno(&e))?;
        ok(0)
    }

    fn unlinkat(&mut self, dirfd: i64, path: u64, flags: u64) -> Result {
        let host = self.resolve(dirfd, path)?;
        let removed = if flags & AT_REMOVEDIR != 0 {
            std::fs::remove_dir(&host)
        } else {
            std::fs::remove_file(&host)
        };
        removed.map_err(|e| io_errno(&e))?;
        ok(0)
    }

    fn renameat(&mut self, old_dirfd: i64, old: u64, new_dirfd: i64, new: u64) -> Result {
        let from = self.resolve(old_dirfd, old)?;
        let to = self.resolve(new_dirfd, new)?;
        std::fs::rename(&from, &to).map_err(|e| io_errno(&e))?;
        ok(0)
    }

    fn getcwd(&mut self, buf: u64, size: u64) -> Result {
        let mut cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned().into_bytes())
            .map_err(|e| io_errno(&e))?;
        cwd.push(0);
        if cwd.len() as u64 > size {
            return Err(-errno::ERANGE);
        }
        self.write_user(buf, &cwd)?;
        ok(cwd.len() as u64)
    }

    fn ioctl(&mut self, fd: i64, request: u64, arg: u64) -> Result {
        if !self.process.files.is_console(fd) {
            return Err(-errno::ENOTTY);
        }
        // The kernel takes the request as 32 bits, so a sign-extended one
        // still names its ioctl.
        match request as u32 {
            // A terminal with default settings, so stdio line-buffers.
            TCGETS => {
                self.write_user(arg, &[0u8; 36])?;
                ok(0)
            }
            TCGETS2 => {
                self.write_user(arg, &[0u8; 44])?;
                ok(0)
            }
            TIOCGWINSZ => {
                let mut winsize = [0u8; 8];
                winsize[..2].copy_from_slice(&24u16.to_le_bytes());
                winsize[2..4].copy_from_slice(&80u16.to_le_bytes());
                self.write_user(arg, &winsize)?;
                ok(0)
            }
            _ => Err(-errno::ENOTTY),
        }
    }

    fn mmap(&mut self, addr: u64, len: u64, prot: u64, flags: u64, fd: i64, offset: u64) -> Result {
        if len == 0 {
            return Err(-errno::EINVAL);
        }
        let hint = (addr != 0).then_some(addr);
        if flags & MAP_FIXED != 0 && (addr & (PAGE_SIZE - 1) != 0 || addr == 0) {
            return Err(-errno::EINVAL);
        }
        let permissions = Permissions::from_prot(prot);
        if flags & MAP_FIXED != 0 {
            self.process.unmap(self.memory, addr, page_ceil(len));
        }
        let start =
            self.process.map_anonymous(self.memory, hint, len, permissions).map_err(fault)?;
        if flags & MAP_FIXED != 0 && start != addr {
            return Err(-errno::ENOMEM);
        }
        if flags & MAP_ANONYMOUS == 0 {
            // A private file mapping is the file's bytes copied in; writes
            // never reach the file.
            let len_usize = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
            let bytes = self.process.files.pread(fd, len_usize, offset)?;
            let writable = permissions.union(Permissions::RW);
            self.process.space.map_range(self.memory, start, len, writable).map_err(fault)?;
            self.write_user(start, &bytes)?;
            self.process.space.map_range(self.memory, start, len, permissions).map_err(fault)?;
            self.process.note_file_map(start, page_ceil(len));
        }
        ok(start)
    }

    fn mremap(&mut self, old_addr: u64, old_len: u64, new_len: u64, flags: u64) -> Result {
        if flags & MREMAP_MAYMOVE == 0 {
            return Err(-errno::ENOMEM);
        }
        let keep = old_len.min(new_len);
        let bytes = self.read_user(old_addr, keep)?;
        let permissions = self.process.space.permissions(old_addr).unwrap_or(Permissions::RW);
        let start =
            self.process.map_anonymous(self.memory, None, new_len, permissions).map_err(fault)?;
        self.write_user(start, &bytes)?;
        self.process.unmap(self.memory, old_addr, old_len);
        ok(start)
    }

    fn mprotect(&mut self, addr: u64, len: u64, prot: u64) -> Result {
        if addr & (PAGE_SIZE - 1) != 0 {
            return Err(-errno::EINVAL);
        }
        if !self.process.space.is_mapped(addr, len) {
            return Err(-errno::ENOMEM);
        }
        self.process
            .space
            .map_range(self.memory, addr, len, Permissions::from_prot(prot))
            .map_err(fault)?;
        ok(0)
    }

    fn zero_if_given(&mut self, addr: u64, len: usize) -> Result {
        if addr != 0 {
            self.write_user(addr, &vec![0u8; len])?;
        }
        ok(0)
    }

    /// Simulated nanoseconds since the run began.
    fn elapsed_ns(&self) -> u128 {
        u128::from(self.cycle) * 1_000_000_000 / u128::from(self.process.clock_hz.max(1))
    }

    fn write_timespec(&mut self, addr: u64, seconds: u64, nanos: u64) -> Result {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&seconds.to_le_bytes());
        bytes[8..].copy_from_slice(&nanos.to_le_bytes());
        self.write_user(addr, &bytes)?;
        ok(0)
    }

    fn clock_gettime(&mut self, clock: u64, addr: u64) -> Result {
        let ns = self.elapsed_ns();
        let epoch = if clock == 0 { REALTIME_EPOCH_SECONDS } else { 0 };
        self.write_timespec(addr, epoch + (ns / 1_000_000_000) as u64, (ns % 1_000_000_000) as u64)
    }

    fn gettimeofday(&mut self, addr: u64) -> Result {
        if addr == 0 {
            return ok(0);
        }
        let us = self.elapsed_ns() / 1000;
        let mut bytes = [0u8; 16];
        bytes[..8]
            .copy_from_slice(&(REALTIME_EPOCH_SECONDS + (us / 1_000_000) as u64).to_le_bytes());
        bytes[8..].copy_from_slice(&((us % 1_000_000) as u64).to_le_bytes());
        self.write_user(addr, &bytes)?;
        ok(0)
    }

    /// Clock ticks (100 per second) the program has run.
    fn ticks(&self) -> u64 {
        (self.elapsed_ns() / 10_000_000) as u64
    }

    fn times(&mut self, addr: u64) -> Result {
        let ticks = self.ticks();
        if addr != 0 {
            let mut bytes = [0u8; 32];
            bytes[..8].copy_from_slice(&ticks.to_le_bytes());
            self.write_user(addr, &bytes)?;
        }
        ok(ticks)
    }

    fn getrusage(&mut self, addr: u64) -> Result {
        let us = self.elapsed_ns() / 1000;
        let mut bytes = [0u8; 144];
        bytes[..8].copy_from_slice(&((us / 1_000_000) as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&((us % 1_000_000) as u64).to_le_bytes());
        self.write_user(addr, &bytes)?;
        ok(0)
    }

    fn uname(&mut self, addr: u64) -> Result {
        let mut bytes = [0u8; 6 * 65];
        for (i, field) in
            ["Linux", "rvsim", "6.6.0", "#1 SMP rvsim", "riscv64", ""].iter().enumerate()
        {
            let start = i * 65;
            bytes[start..start + field.len()].copy_from_slice(field.as_bytes());
        }
        self.write_user(addr, &bytes)?;
        ok(0)
    }

    fn sysinfo(&mut self, addr: u64) -> Result {
        let mut bytes = [0u8; 112];
        bytes[..8].copy_from_slice(&((self.elapsed_ns() / 1_000_000_000) as u64).to_le_bytes());
        bytes[32..40].copy_from_slice(&self.ram_bytes.to_le_bytes());
        bytes[40..48].copy_from_slice(&(self.ram_bytes / 2).to_le_bytes());
        bytes[80..82].copy_from_slice(&1u16.to_le_bytes());
        bytes[104..108].copy_from_slice(&1u32.to_le_bytes());
        self.write_user(addr, &bytes)?;
        ok(0)
    }

    fn prlimit(&mut self, resource: u64, old: u64) -> Result {
        if old != 0 {
            let (cur, max) = match resource {
                RLIMIT_STACK => (self.process.stack_bytes(), RLIM_INFINITY),
                RLIMIT_NOFILE => (1024, 1024),
                _ => (RLIM_INFINITY, RLIM_INFINITY),
            };
            let mut bytes = [0u8; 16];
            bytes[..8].copy_from_slice(&cur.to_le_bytes());
            bytes[8..].copy_from_slice(&max.to_le_bytes());
            self.write_user(old, &bytes)?;
        }
        ok(0)
    }

    fn getrandom(&mut self, buf: u64, len: u64) -> Result {
        let len = usize::try_from(len).map_err(|_| -errno::EINVAL)?;
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            bytes.extend_from_slice(&self.process.next_random().to_le_bytes());
        }
        bytes.truncate(len);
        self.write_user(buf, &bytes)?;
        ok(len as u64)
    }
}

#[allow(clippy::unnecessary_wraps, reason = "the handlers' common result type")]
const fn ok_i64(value: i64) -> Result {
    Ok(Reply::Value(value))
}

/// The flush the retire path applies after a syscall that changed the
/// mappings: every translation, every ASID.
#[must_use]
pub const fn flush_all() -> SfenceVmaInfo {
    SfenceVmaInfo { rs1_idx: RegIdx::new(0), rs2_idx: RegIdx::new(0), rs1_val: 0, rs2_val: 0 }
}

/// The syscall's name for the trace.
#[must_use]
pub const fn name_of(number: u64) -> &'static str {
    match number {
        SYS_GETCWD => "getcwd",
        SYS_FCNTL => "fcntl",
        SYS_IOCTL => "ioctl",
        SYS_MKDIRAT => "mkdirat",
        SYS_UNLINKAT => "unlinkat",
        SYS_RENAMEAT => "renameat",
        SYS_FACCESSAT => "faccessat",
        SYS_OPENAT => "openat",
        SYS_CLOSE => "close",
        SYS_LSEEK => "lseek",
        SYS_READ => "read",
        SYS_WRITE => "write",
        SYS_READV => "readv",
        SYS_WRITEV => "writev",
        SYS_PREAD64 => "pread64",
        SYS_PWRITE64 => "pwrite64",
        SYS_READLINKAT => "readlinkat",
        SYS_NEWFSTATAT => "newfstatat",
        SYS_FSTAT => "fstat",
        SYS_EXIT => "exit",
        SYS_EXIT_GROUP => "exit_group",
        SYS_SET_TID_ADDRESS => "set_tid_address",
        SYS_FUTEX => "futex",
        SYS_SET_ROBUST_LIST => "set_robust_list",
        SYS_NANOSLEEP => "nanosleep",
        SYS_CLOCK_GETTIME => "clock_gettime",
        SYS_CLOCK_GETRES => "clock_getres",
        SYS_CLOCK_NANOSLEEP => "clock_nanosleep",
        SYS_SCHED_YIELD => "sched_yield",
        SYS_KILL => "kill",
        SYS_TKILL => "tkill",
        SYS_TGKILL => "tgkill",
        SYS_RT_SIGACTION => "rt_sigaction",
        SYS_RT_SIGPROCMASK => "rt_sigprocmask",
        SYS_TIMES => "times",
        SYS_UNAME => "uname",
        SYS_GETRUSAGE => "getrusage",
        SYS_GETTIMEOFDAY => "gettimeofday",
        SYS_GETPID => "getpid",
        SYS_GETPPID => "getppid",
        SYS_GETUID => "getuid",
        SYS_GETEUID => "geteuid",
        SYS_GETGID => "getgid",
        SYS_GETEGID => "getegid",
        SYS_GETTID => "gettid",
        SYS_SYSINFO => "sysinfo",
        SYS_BRK => "brk",
        SYS_MUNMAP => "munmap",
        SYS_MREMAP => "mremap",
        SYS_CLONE => "clone",
        SYS_EXECVE => "execve",
        SYS_MMAP => "mmap",
        SYS_MPROTECT => "mprotect",
        SYS_MADVISE => "madvise",
        SYS_RISCV_HWPROBE => "riscv_hwprobe",
        SYS_RISCV_FLUSH_ICACHE => "riscv_flush_icache",
        SYS_PRLIMIT64 => "prlimit64",
        SYS_RENAMEAT2 => "renameat2",
        SYS_GETRANDOM => "getrandom",
        SYS_STATX => "statx",
        SYS_RSEQ => "rseq",
        SYS_CLONE3 => "clone3",
        SYS_FACCESSAT2 => "faccessat2",
        _ => "?",
    }
}
