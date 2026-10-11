//! SE mode's processes, their threads, and the harts the threads hold.
//!
//! Each program is a process whose first thread starts on a hart of its
//! own. A thread keeps its hart until it ends and a hart runs one thread
//! at a time, as in gem5's SE mode: nothing is time-sliced.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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

/// One thread of a process.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Thread {
    tid: Tid,
    hart: HartId,
    /// The address `set_tid_address` gave, for the thread exit protocol.
    clear_child_tid: Option<VirtAddr>,
}

/// What a hart does in SE mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum HartUse {
    /// Parked with no thread.
    Free,
    /// Runs this thread.
    Thread(ThreadRef),
}

/// A process and its threads.
#[derive(Debug)]
struct ThreadGroup {
    process: Process,
    threads: Vec<Thread>,
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
    exit_code: Option<u64>,
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
}

/// The processes' state in a checkpoint.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessesState {
    frames: Option<FrameAllocator>,
    groups: Vec<ThreadGroupState>,
    harts: Vec<HartUse>,
    next_tid: u32,
}

impl Processes {
    /// A table for `hart_count` harts, none of them running anything,
    /// whose processes' frames come from `frames`.
    #[must_use]
    pub fn new(hart_count: usize, frames: FrameAllocator) -> Self {
        Self {
            frames: Some(frames),
            groups: BTreeMap::new(),
            harts: vec![HartUse::Free; hart_count],
            next_tid: 1,
        }
    }

    /// Whether any process is loaded: whether this is SE mode.
    #[must_use]
    pub fn is_se(&self) -> bool {
        !self.groups.is_empty()
    }

    /// Adds `process`, whose first thread runs on `hart`, and returns its
    /// id: 1 for the first program, then up.
    pub fn add_program(&mut self, process: Process, hart: HartId) -> Tid {
        let tgid = self.allocate_tid();
        let thread = Thread { tid: tgid, hart, clear_child_tid: None };
        let group = ThreadGroup { process, threads: vec![thread], exit_code: None };
        let _ = self.groups.insert(tgid, group);
        self.harts[hart.as_index()] = HartUse::Thread(ThreadRef { tgid, tid: tgid });
        tgid
    }

    const fn allocate_tid(&mut self) -> Tid {
        let tid = Tid::new(self.next_tid);
        self.next_tid += 1;
        tid
    }

    /// The thread `hart` runs.
    #[must_use]
    pub fn thread_on(&self, hart: HartId) -> Option<ThreadRef> {
        match self.harts.get(hart.as_index())? {
            HartUse::Thread(thread) => Some(*thread),
            HartUse::Free => None,
        }
    }

    /// Process `tgid` together with the frames it maps from.
    pub fn process_and_frames(&mut self, tgid: Tid) -> Option<(&mut Process, &mut FrameAllocator)> {
        let process = &mut self.groups.get_mut(&tgid)?.process;
        Some((process, self.frames.as_mut()?))
    }

    /// Counts a syscall `number` that process `tgid` made.
    pub fn count_syscall(&mut self, tgid: Tid, number: u64) {
        if let Some(group) = self.groups.get_mut(&tgid) {
            *group.process.syscall_counts.entry(number).or_insert(0) += 1;
        }
    }

    /// Records `thread`'s `set_tid_address` pointer.
    pub fn set_clear_child_tid(&mut self, thread: ThreadRef, addr: Option<VirtAddr>) {
        if let Some(record) =
            self.groups.get_mut(&thread.tgid).and_then(|group| group.thread_mut(thread.tid))
        {
            record.clear_child_tid = addr;
        }
    }

    /// Ends process `tgid` with `code`: its threads are gone and their
    /// harts free. Returns the run's exit code when no process is left
    /// running.
    pub fn exit_process(&mut self, tgid: Tid, code: u64) -> Option<u64> {
        let group = self.groups.get_mut(&tgid)?;
        if group.exit_code.is_some() {
            return None;
        }
        group.exit_code = Some(code);
        for thread in std::mem::take(&mut group.threads) {
            self.harts[thread.hart.as_index()] = HartUse::Free;
        }
        self.every_process_exited().then(|| self.run_exit_code())
    }

    fn every_process_exited(&self) -> bool {
        self.groups.values().all(|group| group.exit_code.is_some())
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
                    exit_code: group.exit_code,
                })
                .collect(),
            harts: self.harts.clone(),
            next_tid: self.next_tid,
        }
    }

    /// The processes a checkpoint holds, each with only the console open.
    #[must_use]
    pub fn from_checkpoint(state: ProcessesState, terminal: &SharedTerminal) -> Self {
        Self {
            frames: state.frames,
            groups: state
                .groups
                .into_iter()
                .map(|group| {
                    let process = Process::from_checkpoint(group.process, terminal.clone());
                    let threads = group.threads;
                    (group.tgid, ThreadGroup { process, threads, exit_code: group.exit_code })
                })
                .collect(),
            harts: state.harts,
            next_tid: state.next_tid,
        }
    }
}
