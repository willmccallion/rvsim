//! SE mode's processes, their threads, and the harts the threads hold.
//!
//! Each program is a process whose first thread starts on a hart of its
//! own; `clone` starts a thread on the lowest-numbered free hart, and
//! fails when there is none. A thread keeps its hart until it ends and a
//! hart runs one thread at a time, as in gem5's SE mode: nothing is
//! time-sliced, and a thread waiting on a futex parks its hart.
//!
//! What a syscall on one hart does to another takes effect at a cycle
//! boundary. A started or woken thread's hart is set going at the end of
//! the cycle ([`Processes::take_wakeups`]); a running thread whose
//! process ended, or whose translations another thread's unmapping made
//! stale, learns of it at its hart's next commit
//! ([`Processes::take_notice`]), as it would from an interrupt.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::files::errno;
use super::loader::UserContext;
use super::{FrameAllocator, Process, ProcessState};
use crate::common::{HartId, VirtAddr};
use crate::soc::terminal::SharedTerminal;

/// A thread's id, as `gettid` returns it; a process's id is its first
/// thread's. Ids are unique across every process of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tid(u32);

impl Tid {
    /// The id `id`.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// The id as a syscall returns it.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0 as u64
    }
}

/// A thread: its process's id and its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRef {
    /// The process's id, which `getpid` returns.
    pub tgid: Tid,
    /// The thread's id, which `gettid` returns.
    pub tid: Tid,
}

/// A thread's wait on a futex word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FutexWait {
    /// The word's address in the process.
    addr: VirtAddr,
    /// A wake reaches the wait when its bitset shares a bit with this.
    bitset: u32,
    /// The cycle the wait times out at, when it has a timeout.
    deadline: Option<u64>,
    /// When the wait began among all waits: a wake takes the oldest first.
    order: u64,
}

/// Where a thread is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
enum ThreadState {
    /// Running on its hart.
    #[default]
    Running,
    /// Its hart is parked until the end of the cycle, when the start or
    /// resume queued for it takes effect.
    Waking,
    /// Waiting on a futex; its hart is parked.
    Waiting(FutexWait),
}

/// One thread of a process.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Thread {
    tid: Tid,
    hart: HartId,
    /// The address `set_tid_address` or `CLONE_CHILD_CLEARTID` gave: zeroed
    /// and woken when the thread exits.
    clear_child_tid: Option<VirtAddr>,
    #[serde(default)]
    state: ThreadState,
    /// Another thread changed the mappings since this one's hart last
    /// dropped its translations.
    #[serde(default)]
    stale_translations: bool,
}

/// What a hart does in SE mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum HartUse {
    /// Parked with no thread.
    Free,
    /// Holds this thread, running or waiting.
    Thread(ThreadRef),
    /// Its thread's process ended from another hart: it parks at its next
    /// commit and is free from then on.
    Stopping,
}

/// A process and its threads.
#[derive(Debug)]
struct ThreadGroup {
    process: Process,
    threads: Vec<Thread>,
    /// The first thread's code once it has called `exit`: the process's
    /// exit code when its last thread ends without an `exit_group`, as in
    /// Linux.
    leader_exit_code: Option<u64>,
    /// The process's exit code, once it has exited.
    exit_code: Option<u64>,
}

impl ThreadGroup {
    fn thread_mut(&mut self, tid: Tid) -> Option<&mut Thread> {
        self.threads.iter_mut().find(|thread| thread.tid == tid)
    }
}

/// A process's state in a checkpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ThreadGroupState {
    tgid: Tid,
    process: ProcessState,
    threads: Vec<Thread>,
    #[serde(default)]
    leader_exit_code: Option<u64>,
    exit_code: Option<u64>,
}

/// What a parked hart takes up at the end of a cycle.
#[derive(Debug)]
pub struct Wakeup {
    /// The hart.
    pub hart: HartId,
    /// What it starts with.
    pub kind: WakeupKind,
    /// Whether the hart drops its translations first: it has a new
    /// thread, or its thread's mappings changed while it waited.
    pub flush_translations: bool,
}

/// How a parked hart starts again.
#[derive(Debug)]
pub enum WakeupKind {
    /// A new thread starts with these registers.
    Start(Box<UserContext>),
    /// A waiting thread resumes with `a0` as its `futex` call's result.
    Resume {
        /// The result.
        a0: u64,
    },
}

/// A wakeup queued during the cycle, for the thread it names.
#[derive(Debug)]
struct QueuedWakeup {
    thread: ThreadRef,
    kind: WakeupKind,
}

/// What a running hart learns at its next commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadNotice {
    /// Its thread has ended: the hart parks.
    Stop,
    /// Its thread's translations may be stale: the hart drops them.
    FlushTranslations,
}

/// The processes SE mode runs and the harts their threads hold, drawing
/// their pages and tables from one frame allocator; empty outside SE
/// mode.
#[derive(Debug, Default)]
pub struct Processes {
    frames: Option<FrameAllocator>,
    /// By process id, which is also the order the programs were loaded in.
    groups: BTreeMap<Tid, ThreadGroup>,
    /// Indexed by hart.
    harts: Vec<HartUse>,
    /// The id the next thread or process gets.
    next_tid: u32,
    /// How many futex waits have begun, to order them.
    waits_begun: u64,
    /// The earliest futex timeout, so a cycle need not look for one.
    next_deadline: Option<u64>,
    /// Whether any hart has a notice to take, so a commit need not look.
    notices_pending: bool,
    /// Starts and resumes queued this cycle.
    wakeups: Vec<QueuedWakeup>,
}

/// The processes' state in a checkpoint, taken between cycles, when no
/// wakeup is queued.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessesState {
    frames: Option<FrameAllocator>,
    groups: Vec<ThreadGroupState>,
    harts: Vec<HartUse>,
    next_tid: u32,
    #[serde(default)]
    waits_begun: u64,
}

impl Processes {
    /// A table for `hart_count` harts, none of them running anything,
    /// whose processes' frames come from `frames`.
    #[must_use]
    pub fn new(hart_count: usize, frames: FrameAllocator) -> Self {
        Self {
            frames: Some(frames),
            harts: vec![HartUse::Free; hart_count],
            next_tid: 1,
            ..Self::default()
        }
    }

    /// Whether any process is loaded: whether this is SE mode.
    #[must_use]
    pub fn is_se(&self) -> bool {
        !self.groups.is_empty()
    }

    /// How many harts the processes' threads can run on.
    #[must_use]
    pub const fn hart_count(&self) -> usize {
        self.harts.len()
    }

    /// Adds `process`, whose first thread runs on `hart`, and returns its
    /// id: 1 for the first program, then up.
    pub fn add_program(&mut self, process: Process, hart: HartId) -> Tid {
        let tgid = self.allocate_tid();
        let thread = Thread {
            tid: tgid,
            hart,
            clear_child_tid: None,
            state: ThreadState::Running,
            stale_translations: false,
        };
        let group =
            ThreadGroup { process, threads: vec![thread], leader_exit_code: None, exit_code: None };
        let _ = self.groups.insert(tgid, group);
        self.harts[hart.as_index()] = HartUse::Thread(ThreadRef { tgid, tid: tgid });
        tgid
    }

    const fn allocate_tid(&mut self) -> Tid {
        let tid = Tid::new(self.next_tid);
        self.next_tid += 1;
        tid
    }

    /// The thread `hart` holds.
    #[must_use]
    pub fn thread_on(&self, hart: HartId) -> Option<ThreadRef> {
        match self.harts.get(hart.as_index())? {
            HartUse::Thread(thread) => Some(*thread),
            HartUse::Free | HartUse::Stopping => None,
        }
    }

    /// The live thread `tid`, of whichever process.
    #[must_use]
    pub fn find_thread(&self, tid: Tid) -> Option<ThreadRef> {
        self.groups.iter().find_map(|(&tgid, group)| {
            group.threads.iter().any(|thread| thread.tid == tid).then_some(ThreadRef { tgid, tid })
        })
    }

    /// Whether process `tgid` exists and has not exited.
    #[must_use]
    pub fn is_running(&self, tgid: Tid) -> bool {
        self.groups.get(&tgid).is_some_and(|group| group.exit_code.is_none())
    }

    /// The ids of the processes still running.
    #[must_use]
    pub fn running_processes(&self) -> Vec<Tid> {
        self.groups
            .iter()
            .filter(|(_, group)| group.exit_code.is_none())
            .map(|(&tgid, _)| tgid)
            .collect()
    }

    /// How many threads are alive across every process.
    #[must_use]
    pub fn live_threads(&self) -> usize {
        self.groups.values().map(|group| group.threads.len()).sum()
    }

    /// Process `tgid`.
    pub fn process_mut(&mut self, tgid: Tid) -> Option<&mut Process> {
        self.groups.get_mut(&tgid).map(|group| &mut group.process)
    }

    /// Process `tgid` together with the frames it maps from.
    pub fn process_and_frames(&mut self, tgid: Tid) -> Option<(&mut Process, &mut FrameAllocator)> {
        let process = &mut self.groups.get_mut(&tgid)?.process;
        Some((process, self.frames.as_mut()?))
    }

    fn thread_mut(&mut self, thread: ThreadRef) -> Option<&mut Thread> {
        self.groups.get_mut(&thread.tgid)?.thread_mut(thread.tid)
    }

    /// Counts a syscall `number` that process `tgid` made.
    pub fn count_syscall(&mut self, tgid: Tid, number: u64) {
        if let Some(group) = self.groups.get_mut(&tgid) {
            *group.process.syscall_counts.entry(number).or_insert(0) += 1;
        }
    }

    /// Records `thread`'s `set_tid_address` pointer.
    pub fn set_clear_child_tid(&mut self, thread: ThreadRef, addr: Option<VirtAddr>) {
        if let Some(record) = self.thread_mut(thread) {
            record.clear_child_tid = addr;
        }
    }

    /// `thread`'s `set_tid_address` pointer.
    #[must_use]
    pub fn clear_child_tid(&self, thread: ThreadRef) -> Option<VirtAddr> {
        self.groups
            .get(&thread.tgid)?
            .threads
            .iter()
            .find(|record| record.tid == thread.tid)?
            .clear_child_tid
    }

    /// Whether `thread`'s process has other threads.
    #[must_use]
    pub fn has_siblings(&self, thread: ThreadRef) -> bool {
        self.groups.get(&thread.tgid).is_some_and(|group| group.threads.len() > 1)
    }

    /// Starts a thread of `parent`'s process with `context` on the
    /// lowest-numbered free hart, from the end of this cycle, and returns
    /// its id; `None` when no hart is free.
    pub fn spawn_thread(
        &mut self,
        parent: ThreadRef,
        context: UserContext,
        clear_child_tid: Option<VirtAddr>,
    ) -> Option<Tid> {
        let index = self.harts.iter().position(|hart| *hart == HartUse::Free)?;
        let hart = HartId::new(u32::try_from(index).ok()?);
        if !self.is_running(parent.tgid) {
            return None;
        }
        let tid = self.allocate_tid();
        let thread = ThreadRef { tgid: parent.tgid, tid };
        let record = Thread {
            tid,
            hart,
            clear_child_tid,
            state: ThreadState::Waking,
            stale_translations: false,
        };
        self.groups.get_mut(&parent.tgid)?.threads.push(record);
        self.harts[index] = HartUse::Thread(thread);
        self.wakeups.push(QueuedWakeup { thread, kind: WakeupKind::Start(Box::new(context)) });
        Some(tid)
    }

    /// Makes `thread` wait on the futex word at `addr` until a wake whose
    /// bitset meets `bitset`, or until cycle `deadline`. Its hart parks.
    pub fn wait(&mut self, thread: ThreadRef, addr: VirtAddr, bitset: u32, deadline: Option<u64>) {
        let order = self.waits_begun;
        self.waits_begun += 1;
        if let Some(record) = self.thread_mut(thread) {
            record.state = ThreadState::Waiting(FutexWait { addr, bitset, deadline, order });
        }
        self.next_deadline = self.earliest_deadline();
        if self.every_thread_waits_forever() {
            tracing::warn!(target: "rvsim::syscall", tid = thread.tid.as_u64(),
                "every thread waits on a futex with no timeout; nothing can wake them");
        }
    }

    /// Wakes up to `count` of process `tgid`'s threads waiting on the
    /// futex word at `addr` whose bitsets meet `bitset`, oldest first, and
    /// returns how many. They resume from the end of this cycle.
    pub fn wake(&mut self, tgid: Tid, addr: VirtAddr, count: usize, bitset: u32) -> usize {
        let Some(group) = self.groups.get_mut(&tgid) else { return 0 };
        let mut waiters: Vec<(u64, usize)> = group
            .threads
            .iter()
            .enumerate()
            .filter_map(|(index, thread)| match thread.state {
                ThreadState::Waiting(wait) if wait.addr == addr && wait.bitset & bitset != 0 => {
                    Some((wait.order, index))
                }
                _ => None,
            })
            .collect();
        waiters.sort_unstable();
        waiters.truncate(count);
        for &(_, index) in &waiters {
            let thread = &mut group.threads[index];
            thread.state = ThreadState::Waking;
            let thread = ThreadRef { tgid, tid: thread.tid };
            self.wakeups.push(QueuedWakeup { thread, kind: WakeupKind::Resume { a0: 0 } });
        }
        if !waiters.is_empty() {
            self.next_deadline = self.earliest_deadline();
        }
        waiters.len()
    }

    /// Times out every futex wait whose deadline is `now` or earlier: the
    /// thread resumes from the end of this cycle with `ETIMEDOUT`.
    pub fn expire_waits(&mut self, now: u64) {
        if self.next_deadline.is_none_or(|deadline| deadline > now) {
            return;
        }
        let timed_out = (-errno::ETIMEDOUT).cast_unsigned();
        let Self { groups, wakeups, .. } = self;
        for (&tgid, group) in groups.iter_mut() {
            for thread in &mut group.threads {
                let ThreadState::Waiting(wait) = thread.state else { continue };
                if wait.deadline.is_some_and(|deadline| deadline <= now) {
                    thread.state = ThreadState::Waking;
                    let thread = ThreadRef { tgid, tid: thread.tid };
                    wakeups
                        .push(QueuedWakeup { thread, kind: WakeupKind::Resume { a0: timed_out } });
                }
            }
        }
        self.next_deadline = self.earliest_deadline();
    }

    /// The earliest cycle a futex wait times out at.
    #[must_use]
    pub const fn next_deadline(&self) -> Option<u64> {
        self.next_deadline
    }

    fn earliest_deadline(&self) -> Option<u64> {
        self.waits().filter_map(|wait| wait.deadline).min()
    }

    fn waits(&self) -> impl Iterator<Item = FutexWait> + '_ {
        self.groups.values().flat_map(|group| &group.threads).filter_map(|thread| {
            match thread.state {
                ThreadState::Waiting(wait) => Some(wait),
                ThreadState::Running | ThreadState::Waking => None,
            }
        })
    }

    fn every_thread_waits_forever(&self) -> bool {
        let mut threads = self.groups.values().flat_map(|group| &group.threads).peekable();
        threads.peek().is_some()
            && threads.all(|thread| {
                matches!(thread.state, ThreadState::Waiting(FutexWait { deadline: None, .. }))
            })
    }

    /// Ends `thread` alone, as `exit` does: its hart, which it parks, is
    /// free. When it was its process's last thread the process ends too,
    /// with its first thread's exit code; then, when no process is left
    /// running, the run's exit code comes back.
    pub fn exit_thread(&mut self, thread: ThreadRef, code: u64) -> Option<u64> {
        let group = self.groups.get_mut(&thread.tgid)?;
        let index = group.threads.iter().position(|record| record.tid == thread.tid)?;
        let ended = group.threads.remove(index);
        if thread.tid == thread.tgid {
            group.leader_exit_code = Some(code);
        }
        let process_code = group.threads.is_empty().then(|| group.leader_exit_code.unwrap_or(code));
        self.harts[ended.hart.as_index()] = HartUse::Free;
        self.exit_process(thread.tgid, process_code?, None)
    }

    /// Ends process `tgid` with `code`, as `exit_group` or a fatal signal
    /// does. `caller`, the thread that ended it when it is one of the
    /// process's, parks its own hart; another running thread's hart stops
    /// at its next commit, and a parked one's is free at once. Returns the
    /// run's exit code when no process is left running.
    pub fn exit_process(&mut self, tgid: Tid, code: u64, caller: Option<Tid>) -> Option<u64> {
        let group = self.groups.get_mut(&tgid)?;
        if group.exit_code.is_some() {
            return None;
        }
        group.exit_code = Some(code);
        for thread in std::mem::take(&mut group.threads) {
            let running_elsewhere =
                thread.state == ThreadState::Running && Some(thread.tid) != caller;
            self.harts[thread.hart.as_index()] =
                if running_elsewhere { HartUse::Stopping } else { HartUse::Free };
            self.notices_pending |= running_elsewhere;
        }
        self.wakeups.retain(|wakeup| wakeup.thread.tgid != tgid);
        self.next_deadline = self.earliest_deadline();
        self.every_process_exited().then(|| self.run_exit_code())
    }

    fn every_process_exited(&self) -> bool {
        self.groups.values().all(|group| group.exit_code.is_some())
    }

    /// Frees `hart` when it was stopping: its thread's process has ended
    /// and the hart has now parked.
    pub fn release_stopping(&mut self, hart: HartId) {
        if let Some(slot @ HartUse::Stopping) = self.harts.get_mut(hart.as_index()) {
            *slot = HartUse::Free;
            self.notices_pending = self.any_notice();
        }
    }

    /// Marks the translations of every thread of `changed_by`'s process
    /// but `changed_by` stale: it removed or changed a mapping their
    /// harts' TLBs may hold.
    pub fn mark_translations_stale(&mut self, changed_by: ThreadRef) {
        let Some(group) = self.groups.get_mut(&changed_by.tgid) else { return };
        for thread in group.threads.iter_mut().filter(|thread| thread.tid != changed_by.tid) {
            thread.stale_translations = true;
            self.notices_pending = true;
        }
    }

    /// What `hart` has to learn at this commit, taken: that its thread has
    /// ended, or that its running thread's translations are stale.
    pub fn take_notice(&mut self, hart: HartId) -> Option<ThreadNotice> {
        if !self.notices_pending {
            return None;
        }
        let notice = match *self.harts.get(hart.as_index())? {
            HartUse::Free => None,
            HartUse::Stopping => {
                self.harts[hart.as_index()] = HartUse::Free;
                Some(ThreadNotice::Stop)
            }
            HartUse::Thread(thread) => self
                .thread_mut(thread)
                .filter(|record| record.state == ThreadState::Running && record.stale_translations)
                .map(|record| {
                    record.stale_translations = false;
                    ThreadNotice::FlushTranslations
                }),
        };
        if notice.is_some() {
            self.notices_pending = self.any_notice();
        }
        notice
    }

    fn any_notice(&self) -> bool {
        self.harts.contains(&HartUse::Stopping)
            || self
                .groups
                .values()
                .flat_map(|group| &group.threads)
                .any(|thread| thread.stale_translations)
    }

    /// The starts and resumes this cycle queued for threads still alive,
    /// taken; each such thread is running from now on.
    pub fn take_wakeups(&mut self) -> Vec<Wakeup> {
        let queued = std::mem::take(&mut self.wakeups);
        let wakeups: Vec<Wakeup> = queued
            .into_iter()
            .filter_map(|wakeup| {
                let record = self.thread_mut(wakeup.thread)?;
                if record.state != ThreadState::Waking {
                    return None;
                }
                record.state = ThreadState::Running;
                let new_thread = matches!(wakeup.kind, WakeupKind::Start(_));
                let stale = std::mem::take(&mut record.stale_translations);
                Some(Wakeup {
                    hart: record.hart,
                    kind: wakeup.kind,
                    flush_translations: new_thread || stale,
                })
            })
            .collect();
        if !wakeups.is_empty() {
            self.notices_pending = self.any_notice();
        }
        wakeups
    }

    /// Each program's exit code in the order they were loaded: `None` for
    /// one still running.
    #[must_use]
    pub fn exit_codes(&self) -> Vec<Option<u64>> {
        self.groups.values().map(|group| group.exit_code).collect()
    }

    /// The run's exit code once every program has exited: the first
    /// non-zero code in load order, else 0.
    #[must_use]
    pub fn run_exit_code(&self) -> u64 {
        self.exit_codes().into_iter().flatten().find(|&code| code != 0).unwrap_or(0)
    }

    /// How many host files the processes have open between them.
    #[must_use]
    pub fn host_files_open(&self) -> usize {
        self.groups.values().map(|group| group.process.files.host_files_open()).sum()
    }

    /// The state a checkpoint keeps.
    #[must_use]
    pub fn checkpoint(&self) -> ProcessesState {
        ProcessesState {
            frames: self.frames.clone(),
            groups: self
                .groups
                .iter()
                .map(|(&tgid, group)| ThreadGroupState {
                    tgid,
                    process: group.process.checkpoint(),
                    threads: group.threads.clone(),
                    leader_exit_code: group.leader_exit_code,
                    exit_code: group.exit_code,
                })
                .collect(),
            harts: self.harts.clone(),
            next_tid: self.next_tid,
            waits_begun: self.waits_begun,
        }
    }

    /// The processes a checkpoint holds, each with only the console open.
    #[must_use]
    pub fn from_checkpoint(state: ProcessesState, terminal: &SharedTerminal) -> Self {
        let groups = state
            .groups
            .into_iter()
            .map(|group| {
                let process = Process::from_checkpoint(group.process, terminal.clone());
                let restored = ThreadGroup {
                    process,
                    threads: group.threads,
                    leader_exit_code: group.leader_exit_code,
                    exit_code: group.exit_code,
                };
                (group.tgid, restored)
            })
            .collect();
        let mut processes = Self {
            frames: state.frames,
            groups,
            harts: state.harts,
            next_tid: state.next_tid,
            waits_begun: state.waits_begun,
            ..Self::default()
        };
        processes.next_deadline = processes.earliest_deadline();
        processes.notices_pending = processes.any_notice();
        processes
    }
}
