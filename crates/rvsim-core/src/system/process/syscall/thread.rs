//! The syscalls about threads and processes as such, rather than one
//! process's memory, files and clock: ids, `clone`, futexes, `exit` and
//! `exit_group`, signals, and the CPUs a thread may run on. They follow
//! Linux 6.6 (`kernel/fork.c`, `kernel/futex/`, `kernel/signal.c`,
//! `kernel/sched/core.c`).

use super::{
    REALTIME_EPOCH_SECONDS, Reply, Result, SYS_CLONE, SYS_EXIT, SYS_EXIT_GROUP, SYS_FUTEX,
    SYS_GETPID, SYS_GETTID, SYS_KILL, SYS_SCHED_GETAFFINITY, SYS_SET_TID_ADDRESS, SYS_TGKILL,
    SYS_TKILL, ok, user_pointer,
};
use crate::arch::Hart;
use crate::common::VirtAddr;
use crate::common::trace::Hex;
use crate::isa::reg;
use crate::sim::memory::GlobalMemory;
use crate::system::process::files::errno;
use crate::system::process::loader::UserContext;
use crate::system::process::{Process, Processes, ThreadRef, Tid};

const CSIGNAL: u64 = 0xff;
const CLONE_VM: u64 = 0x100;
const CLONE_FS: u64 = 0x200;
const CLONE_FILES: u64 = 0x400;
const CLONE_SIGHAND: u64 = 0x800;
const CLONE_THREAD: u64 = 0x1_0000;
const CLONE_SYSVSEM: u64 = 0x4_0000;
const CLONE_SETTLS: u64 = 0x8_0000;
const CLONE_PARENT_SETTID: u64 = 0x10_0000;
const CLONE_CHILD_CLEARTID: u64 = 0x20_0000;
const CLONE_DETACHED: u64 = 0x40_0000;
const CLONE_CHILD_SETTID: u64 = 0x100_0000;
/// What a thread sharing everything with its process passes, as glibc's
/// `pthread_create` does.
const CLONE_THREAD_SHARES: u64 = CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD;
/// Every flag SE mode serves: a thread's, its TLS and its id pointers.
const CLONE_SERVED: u64 = CLONE_THREAD_SHARES
    | CSIGNAL
    | CLONE_SYSVSEM
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_DETACHED
    | CLONE_CHILD_SETTID;

const FUTEX_WAIT: u32 = 0;
const FUTEX_WAKE: u32 = 1;
const FUTEX_WAIT_BITSET: u32 = 9;
const FUTEX_WAKE_BITSET: u32 = 10;
const FUTEX_PRIVATE_FLAG: u32 = 128;
const FUTEX_CLOCK_REALTIME: u32 = 256;
/// The bitset a plain `FUTEX_WAIT` or `FUTEX_WAKE` uses: every bit.
const FUTEX_BITSET_MATCH_ANY: u32 = u32::MAX;
const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// The highest signal number, `_NSIG`.
const MAX_SIGNAL: u64 = 64;
/// The signals whose default action is to ignore them: `SIGCHLD`,
/// `SIGCONT`, `SIGURG` and `SIGWINCH`.
const IGNORED_BY_DEFAULT: [u64; 4] = [17, 18, 23, 28];
/// The first program's process id, which `kill(-1, ...)` leaves out as
/// Linux leaves out init.
const INIT: Tid = Tid::new(1);
/// The CPUs a 6.6 kernel is built for, which sizes its CPU masks.
const NR_CPUS: usize = 64;

/// What a thread-level syscall works on.
pub(super) struct ThreadCall<'a> {
    pub(super) processes: &'a mut Processes,
    pub(super) memory: &'a mut GlobalMemory,
    /// The calling thread.
    pub(super) caller: ThreadRef,
    /// Its hart, whose registers a cloned thread copies.
    pub(super) hart: &'a Hart,
    /// The `ecall`'s address.
    pub(super) pc: VirtAddr,
    pub(super) cycle: u64,
}

impl ThreadCall<'_> {
    /// Serves syscall `number` when it is one of these; `None` for any
    /// other.
    pub(super) fn dispatch(&mut self, number: u64, a: [u64; 6]) -> Option<Result> {
        let result = match number {
            SYS_GETPID => ok(self.caller.tgid.as_u64()),
            SYS_GETTID => ok(self.caller.tid.as_u64()),
            SYS_SET_TID_ADDRESS => {
                self.processes.set_clear_child_tid(self.caller, user_pointer(a[0]));
                ok(self.caller.tid.as_u64())
            }
            SYS_CLONE => self.clone(a[0], user_pointer(a[1]), VirtAddr::new(a[2]), a[3], a[4]),
            SYS_EXIT => Ok(self.exit(a[0] & 0xff)),
            SYS_EXIT_GROUP => Ok(self.exit_group(a[0] & 0xff)),
            SYS_FUTEX => self.futex(VirtAddr::new(a[0]), a[1] as u32, a[2] as u32, a[3], a[5]),
            SYS_KILL => self.kill(a[0] as i32, a[1]),
            SYS_TKILL => self.tkill(a[0] as i32, a[1]),
            SYS_TGKILL => self.tgkill(a[0] as i32, a[1] as i32, a[2]),
            SYS_SCHED_GETAFFINITY => {
                self.sched_getaffinity(a[0] as i32, a[1] as u32, VirtAddr::new(a[2]))
            }
            _ => return None,
        };
        Some(result)
    }

    /// The caller's process, and RAM.
    fn process_and_memory(
        &mut self,
    ) -> std::result::Result<(&mut Process, &mut GlobalMemory), i64> {
        let process = self.processes.process_mut(self.caller.tgid).ok_or(-errno::ESRCH)?;
        Ok((process, &mut *self.memory))
    }

    /// Stores `value` as a 32-bit word at `addr` in the caller's process
    /// as the kernel's `put_user` does for the thread protocol: a fault is
    /// ignored.
    fn put_word(&mut self, addr: VirtAddr, value: u32) {
        if let Ok((process, memory)) = self.process_and_memory() {
            let _ = process.space.write(memory, addr, &value.to_le_bytes());
        }
    }

    /// `clone(flags, stack, parent_tid, tls, child_tid)`, in riscv's
    /// argument order: a thread of the caller's process starts on a free
    /// hart with a copy of the caller's registers, `a0` 0, `stack` as its
    /// stack pointer when given and `tls` as its thread pointer with
    /// `CLONE_SETTLS`. SE mode runs one thread per hart, so with no hart
    /// free the call fails with `EAGAIN`; a new process (`fork`) is not
    /// served.
    fn clone(
        &mut self,
        flags: u64,
        stack: Option<VirtAddr>,
        parent_tid: VirtAddr,
        tls: u64,
        child_tid: u64,
    ) -> Result {
        let has = |flag: u64| flags & flag != 0;
        if has(CLONE_THREAD) && !has(CLONE_SIGHAND) || has(CLONE_SIGHAND) && !has(CLONE_VM) {
            return Err(-errno::EINVAL);
        }
        if flags & CLONE_THREAD_SHARES != CLONE_THREAD_SHARES || flags & !CLONE_SERVED != 0 {
            tracing::warn!(target: "rvsim::syscall", flags = %Hex(flags),
                "clone that is not a thread sharing its process's memory, files and signal \
                 handlers: SE mode starts threads only; returning ENOSYS");
            return Err(-errno::ENOSYS);
        }
        let mut context = UserContext::of(self.hart, self.pc.wrapping_add(4));
        context.set(reg::REG_A0, 0);
        if let Some(stack) = stack {
            context.set(reg::REG_SP, stack.val());
        }
        if has(CLONE_SETTLS) {
            context.set(reg::REG_TP, tls);
        }
        let child_tid = user_pointer(child_tid);
        let clear_child_tid = child_tid.filter(|_| has(CLONE_CHILD_CLEARTID));
        let Some(tid) = self.processes.spawn_thread(self.caller, context, clear_child_tid) else {
            tracing::warn!(target: "rvsim::syscall", harts = self.processes.hart_count(),
                "clone with no hart free: SE mode runs one thread per hart; returning EAGAIN");
            return Err(-errno::EAGAIN);
        };
        let tid_word = tid.as_u64() as u32;
        if has(CLONE_PARENT_SETTID) {
            self.put_word(parent_tid, tid_word);
        }
        if let Some(child_tid) = child_tid.filter(|_| has(CLONE_CHILD_SETTID)) {
            self.put_word(child_tid, tid_word);
        }
        ok(tid.as_u64())
    }

    /// `exit`: the calling thread ends. When others share its memory, its
    /// `set_tid_address` word is zeroed and one waiter on it woken, which
    /// is how `pthread_join` learns of it.
    fn exit(&mut self, code: u64) -> Reply {
        if let Some(clear_child_tid) = self.processes.clear_child_tid(self.caller)
            && self.processes.has_siblings(self.caller)
        {
            self.put_word(clear_child_tid, 0);
            let tgid = self.caller.tgid;
            let _ = self.processes.wake(tgid, clear_child_tid, 1, FUTEX_BITSET_MATCH_ANY);
        }
        Reply::Ended(self.processes.exit_thread(self.caller, code))
    }

    /// `exit_group`: every thread of the caller's process ends.
    fn exit_group(&mut self, code: u64) -> Reply {
        Reply::Ended(self.processes.exit_process(self.caller.tgid, code, Some(self.caller.tid)))
    }

    /// `futex(addr, op, val, timeout, addr2, val3)`: wait and wake, with
    /// their bitset forms, private or not. Requeueing and the
    /// priority-inheritance operations are not served.
    fn futex(&mut self, addr: VirtAddr, op: u32, val: u32, timeout: u64, val3: u64) -> Result {
        let cmd = op & !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);
        let realtime = op & FUTEX_CLOCK_REALTIME != 0;
        let deadline = match user_pointer(timeout) {
            Some(timeout) if matches!(cmd, FUTEX_WAIT | FUTEX_WAIT_BITSET) => {
                Some(self.futex_deadline(cmd, realtime, timeout)?)
            }
            _ => None,
        };
        if realtime && cmd != FUTEX_WAIT_BITSET {
            return Err(-errno::ENOSYS);
        }
        let shared = op & FUTEX_PRIVATE_FLAG == 0;
        let bitset = val3 as u32;
        match cmd {
            FUTEX_WAIT => self.futex_wait(addr, val, FUTEX_BITSET_MATCH_ANY, deadline),
            FUTEX_WAIT_BITSET => self.futex_wait(addr, val, bitset, deadline),
            FUTEX_WAKE => self.futex_wake(addr, val, FUTEX_BITSET_MATCH_ANY, shared),
            FUTEX_WAKE_BITSET => self.futex_wake(addr, val, bitset, shared),
            _ => {
                tracing::warn!(target: "rvsim::syscall", cmd,
                    "futex operation SE mode does not serve; returning ENOSYS");
                Err(-errno::ENOSYS)
            }
        }
    }

    /// The cycle a wait with the `timespec` at `timeout` gives up at:
    /// `FUTEX_WAIT`'s is relative to now, `FUTEX_WAIT_BITSET`'s an
    /// absolute time on the monotonic clock, or on the realtime clock with
    /// `FUTEX_CLOCK_REALTIME`.
    fn futex_deadline(
        &mut self,
        cmd: u32,
        realtime: bool,
        timeout: VirtAddr,
    ) -> std::result::Result<u64, i64> {
        let cycle = self.cycle;
        let (process, memory) = self.process_and_memory()?;
        let read = |at: VirtAddr| process.space.read_u64(memory, at).map_err(|_| -errno::EFAULT);
        let seconds = read(timeout)? as i64;
        let nanos = read(timeout.wrapping_add(8))? as i64;
        if seconds < 0 || !(0..NANOS_PER_SECOND as i64).contains(&nanos) {
            return Err(-errno::EINVAL);
        }
        let ns = seconds as u128 * NANOS_PER_SECOND + nanos as u128;
        let monotonic = if cmd == FUTEX_WAIT {
            process.nanoseconds_at(cycle) + ns
        } else if realtime {
            ns.saturating_sub(u128::from(REALTIME_EPOCH_SECONDS) * NANOS_PER_SECOND)
        } else {
            ns
        };
        Ok(process.cycle_at_nanoseconds(monotonic))
    }

    /// Waits while the word at `addr` holds `expected`, until a wake whose
    /// bitset meets `bitset` or until cycle `deadline`.
    fn futex_wait(
        &mut self,
        addr: VirtAddr,
        expected: u32,
        bitset: u32,
        deadline: Option<u64>,
    ) -> Result {
        if bitset == 0 || !addr.val().is_multiple_of(4) {
            return Err(-errno::EINVAL);
        }
        let (process, memory) = self.process_and_memory()?;
        let bytes = process.space.read(memory, addr, 4).map_err(|_| -errno::EFAULT)?;
        let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if word != expected {
            return Err(-errno::EAGAIN);
        }
        if deadline.is_some_and(|deadline| deadline <= self.cycle) {
            return Err(-errno::ETIMEDOUT);
        }
        self.processes.wait(self.caller, addr, bitset, deadline);
        Ok(Reply::Wait)
    }

    /// Wakes up to `count` waiters on the word at `addr` whose bitsets
    /// meet `bitset`, and at least one, as Linux does for a count of zero
    /// or less. A shared futex must be mapped.
    fn futex_wake(&mut self, addr: VirtAddr, count: u32, bitset: u32, shared: bool) -> Result {
        if bitset == 0 || !addr.val().is_multiple_of(4) {
            return Err(-errno::EINVAL);
        }
        if shared && self.process_and_memory()?.0.space.translate(addr).is_none() {
            return Err(-errno::EFAULT);
        }
        let count = (count as i32).max(1) as usize;
        ok(self.processes.wake(self.caller.tgid, addr, count, bitset) as u64)
    }

    /// `kill(pid, signal)`. Each program is its own process group, so a
    /// `pid` of 0 is the caller's process and `-pid` names process `pid`;
    /// -1 is every other process but the first program's.
    fn kill(&mut self, pid: i32, signal: u64) -> Result {
        let targets = match pid {
            0 => vec![self.caller.tgid],
            -1 => self
                .processes
                .running_processes()
                .into_iter()
                .filter(|&tgid| tgid != INIT && tgid != self.caller.tgid)
                .collect(),
            pid => {
                let tgid = Tid::new(pid.unsigned_abs());
                if self.processes.is_running(tgid) { vec![tgid] } else { Vec::new() }
            }
        };
        if targets.is_empty() {
            return Err(-errno::ESRCH);
        }
        self.signal(&targets, signal)
    }

    /// `tkill(tid, signal)`: to the process of thread `tid`.
    fn tkill(&mut self, tid: i32, signal: u64) -> Result {
        let tid = u32::try_from(tid).ok().filter(|&tid| tid > 0).ok_or(-errno::EINVAL)?;
        let thread = self.processes.find_thread(Tid::new(tid)).ok_or(-errno::ESRCH)?;
        self.signal(&[thread.tgid], signal)
    }

    /// `tgkill(tgid, tid, signal)`: to the process of thread `tid`, which
    /// must belong to process `tgid`.
    fn tgkill(&mut self, tgid: i32, tid: i32, signal: u64) -> Result {
        let positive = |id: i32| u32::try_from(id).ok().filter(|&id| id > 0).map(Tid::new);
        let (Some(tgid), Some(tid)) = (positive(tgid), positive(tid)) else {
            return Err(-errno::EINVAL);
        };
        let thread = self
            .processes
            .find_thread(tid)
            .filter(|thread| thread.tgid == tgid)
            .ok_or(-errno::ESRCH)?;
        self.signal(&[thread.tgid], signal)
    }

    /// Delivers `signal` to the processes `targets` by its default action,
    /// handlers never running: 0 only checks that they exist, a signal
    /// whose default is to be ignored does nothing, and any other ends
    /// each process with the shell's status for it, 128 plus the signal.
    fn signal(&mut self, targets: &[Tid], signal: u64) -> Result {
        if signal > MAX_SIGNAL {
            return Err(-errno::EINVAL);
        }
        if signal == 0 || IGNORED_BY_DEFAULT.contains(&signal) {
            return ok(0);
        }
        let mut caller_ended = None;
        for &tgid in targets {
            let caller = (tgid == self.caller.tgid).then_some(self.caller.tid);
            let run_exit = self.processes.exit_process(tgid, 128 + signal, caller);
            if caller.is_some() {
                caller_ended = Some(run_exit);
            }
        }
        caller_ended.map_or_else(|| ok(0), |run_exit| Ok(Reply::Ended(run_exit)))
    }

    /// `sched_getaffinity(pid, len, mask)`: every hart, since a thread may
    /// start on any of them. The mask is the size a 6.6 kernel's is.
    fn sched_getaffinity(&mut self, pid: i32, len: u32, mask: VirtAddr) -> Result {
        let harts = self.processes.hart_count();
        let len = len as usize;
        if len * 8 < harts || !len.is_multiple_of(size_of::<u64>()) {
            return Err(-errno::EINVAL);
        }
        let known = pid == 0
            || u32::try_from(pid)
                .is_ok_and(|tid| self.processes.find_thread(Tid::new(tid)).is_some());
        if !known {
            return Err(-errno::ESRCH);
        }
        let bytes = len.min(harts.max(NR_CPUS).div_ceil(64) * size_of::<u64>());
        let mut bits = vec![0u8; bytes];
        for hart in 0..harts {
            bits[hart / 8] |= 1 << (hart % 8);
        }
        let (process, memory) = self.process_and_memory()?;
        process.space.write(memory, mask, &bits).map_err(|_| -errno::EFAULT)?;
        ok(bytes as u64)
    }
}
