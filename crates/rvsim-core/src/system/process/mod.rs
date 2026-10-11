//! SE mode: Linux user programs run without a kernel, one per hart.
//!
//! The simulator loads each static ELF, builds its page tables and stack,
//! and runs it in user mode on its hart; an `ecall` retires as an
//! instruction whose effect is the syscall, served by the simulator
//! against the host's files, clock and memory, as gem5's SE mode does.
//! Syscalls cost the pipeline a flush and nothing else; there is no
//! kernel to charge for. The programs share RAM's frames and the caches
//! below their cores, and the run ends when the last of them exits.

pub(crate) mod files;
pub mod loader;
mod page_table;
pub(crate) mod syscall;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::HartId;
use crate::isa::privileged::PagingMode;
use crate::sim::memory::GlobalMemory;
use crate::soc::terminal::SharedTerminal;

pub use page_table::{AddressSpace, FrameAllocator, PAGE_SIZE, Permissions, SpaceError};

use files::FileTable;

/// The host-side state of the running program.
#[derive(Debug)]
pub struct Process {
    /// Its page tables and mappings.
    pub space: AddressSpace,
    /// Its descriptors.
    pub(crate) files: FileTable,
    /// Where `brk` started: the end of the loaded image, page-aligned.
    brk_start: u64,
    /// The current program break.
    brk: u64,
    /// Where the next anonymous `mmap` without an address goes.
    mmap_next: u64,
    /// The `set_tid_address` pointer, for the thread exit protocol.
    pub clear_child_tid: u64,
    /// The core clock the time syscalls divide cycles by.
    pub clock_hz: u64,
    /// Deterministic state for `getrandom`.
    random: u64,
    /// `mmap`ed file ranges, so `munmap` can forget them: start to length.
    file_maps: BTreeMap<u64, u64>,
    /// How many syscalls were served, by number.
    pub syscall_counts: BTreeMap<u64, u64>,
    /// The process id the program sees: 1 for the first program, then up.
    pub pid: u64,
}

/// A process's state in a checkpoint: everything but its host files,
/// which a restored run does not get back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessState {
    pid: u64,
    space: AddressSpace,
    brk_start: u64,
    brk: u64,
    mmap_next: u64,
    clear_child_tid: u64,
    clock_hz: u64,
    random: u64,
    file_maps: BTreeMap<u64, u64>,
    syscall_counts: BTreeMap<u64, u64>,
}

/// Where a process's regions start, relative to the address space's top.
struct Layout {
    stack_top: u64,
    stack_bytes: u64,
    mmap_base: u64,
}

impl Layout {
    const STACK_BYTES: u64 = 8 << 20;

    /// Linux's layout for the mode: the stack at the top of user space,
    /// anonymous mappings from the middle up, the heap after the image.
    const fn for_space(space: &AddressSpace) -> Self {
        let top = space.user_top();
        Self { stack_top: top, stack_bytes: Self::STACK_BYTES, mmap_base: top / 2 }
    }
}

impl Process {
    /// Process `pid` over an address space whose heap starts at `brk_start`.
    #[must_use]
    pub fn new(
        space: AddressSpace,
        brk_start: u64,
        clock_hz: u64,
        terminal: SharedTerminal,
        pid: u64,
    ) -> Self {
        let layout = Layout::for_space(&space);
        Self {
            pid,
            space,
            files: FileTable::new(terminal),
            brk_start,
            brk: brk_start,
            mmap_next: layout.mmap_base,
            clear_child_tid: 0,
            clock_hz,
            random: 0x9E37_79B9_7F4A_7C15,
            file_maps: BTreeMap::new(),
            syscall_counts: BTreeMap::new(),
        }
    }

    /// The state a checkpoint keeps.
    #[must_use]
    pub fn checkpoint(&self) -> ProcessState {
        ProcessState {
            pid: self.pid,
            space: self.space.clone(),
            brk_start: self.brk_start,
            brk: self.brk,
            mmap_next: self.mmap_next,
            clear_child_tid: self.clear_child_tid,
            clock_hz: self.clock_hz,
            random: self.random,
            file_maps: self.file_maps.clone(),
            syscall_counts: self.syscall_counts.clone(),
        }
    }

    /// The process a checkpoint holds, with only the console open.
    #[must_use]
    pub fn from_checkpoint(state: ProcessState, terminal: SharedTerminal) -> Self {
        Self {
            pid: state.pid,
            space: state.space,
            files: FileTable::new(terminal),
            brk_start: state.brk_start,
            brk: state.brk,
            mmap_next: state.mmap_next,
            clear_child_tid: state.clear_child_tid,
            clock_hz: state.clock_hz,
            random: state.random,
            file_maps: state.file_maps,
            syscall_counts: state.syscall_counts,
        }
    }

    /// The paging mode the program runs under.
    #[must_use]
    pub const fn paging_mode(&self) -> PagingMode {
        self.space.mode()
    }

    /// The current program break.
    #[must_use]
    pub const fn brk(&self) -> u64 {
        self.brk
    }

    /// Moves the program break to `new_brk`, mapping or unmapping the
    /// pages between, and returns where it ends up: unchanged when the
    /// request is below the start or cannot be mapped, as Linux does.
    pub fn set_brk(
        &mut self,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
        new_brk: u64,
    ) -> u64 {
        if new_brk < self.brk_start || !self.space.in_range(new_brk) {
            return self.brk;
        }
        let old_end = page_ceil(self.brk);
        let new_end = page_ceil(new_brk);
        if new_end > old_end {
            if self
                .space
                .map_range(memory, frames, old_end, new_end - old_end, Permissions::RW)
                .is_err()
            {
                return self.brk;
            }
        } else if new_end < old_end {
            self.space.unmap_range(memory, new_end, old_end - new_end);
        }
        self.brk = new_brk;
        self.brk
    }

    /// Maps `len` bytes of fresh zeroed memory with `permissions` at
    /// `hint` when it is given and free, otherwise at the next free place.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when RAM is full or the address space is.
    pub fn map_anonymous(
        &mut self,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
        hint: Option<u64>,
        len: u64,
        permissions: Permissions,
    ) -> Result<u64, SpaceError> {
        let len = page_ceil(len);
        let start = match hint {
            Some(address)
                if address & (PAGE_SIZE - 1) == 0 && !self.space.is_mapped(address, len) =>
            {
                address
            }
            _ => {
                let start = self.mmap_next;
                self.mmap_next = start.checked_add(len).ok_or(SpaceError::OutOfRange(start))?;
                start
            }
        };
        self.space.map_range(memory, frames, start, len, permissions)?;
        Ok(start)
    }

    /// Unmaps `[start, start + len)`.
    pub fn unmap(&mut self, memory: &mut GlobalMemory, start: u64, len: u64) {
        self.space.unmap_range(memory, start, len);
        let _ = self.file_maps.remove(&start);
    }

    /// Records a file-backed mapping so `munmap` can forget it.
    pub fn note_file_map(&mut self, start: u64, len: u64) {
        let _ = self.file_maps.insert(start, len);
    }

    /// The next pseudo-random 64 bits, reproducible run to run.
    pub const fn next_random(&mut self) -> u64 {
        let mut x = self.random;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.random = x;
        x
    }

    /// Where the stack's top page is, for the loader.
    #[must_use]
    pub const fn stack_top(&self) -> u64 {
        Layout::for_space(&self.space).stack_top
    }

    /// How much stack the loader maps below the top.
    #[must_use]
    pub const fn stack_bytes(&self) -> u64 {
        Layout::for_space(&self.space).stack_bytes
    }
}

/// `value` rounded up to a page boundary.
#[must_use]
pub const fn page_ceil(value: u64) -> u64 {
    (value + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// A hart's program: the process, and its exit code once it has exited.
#[derive(Debug)]
struct Slot {
    process: Process,
    exit_code: Option<u64>,
}

/// The user programs SE mode runs, one per hart, drawing their pages and
/// tables from one frame allocator; empty outside SE mode.
#[derive(Debug, Default)]
pub struct Processes {
    frames: Option<FrameAllocator>,
    /// Indexed by hart.
    slots: Vec<Option<Slot>>,
}

/// The programs' state in a checkpoint.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessesState {
    frames: Option<FrameAllocator>,
    slots: Vec<Option<(ProcessState, Option<u64>)>>,
}

impl Processes {
    /// A table for `hart_count` harts whose programs' frames come from
    /// `frames`.
    #[must_use]
    pub fn new(hart_count: usize, frames: FrameAllocator) -> Self {
        Self { frames: Some(frames), slots: (0..hart_count).map(|_| None).collect() }
    }

    /// Whether any program is loaded: whether this is SE mode.
    #[must_use]
    pub fn is_se(&self) -> bool {
        self.slots.iter().any(Option::is_some)
    }

    /// Gives `hart` the program `process`.
    pub fn insert(&mut self, hart: HartId, process: Process) {
        self.slots[hart.as_index()] = Some(Slot { process, exit_code: None });
    }

    /// The frames every program draws from, while in SE mode.
    pub const fn frames_mut(&mut self) -> Option<&mut FrameAllocator> {
        self.frames.as_mut()
    }

    /// `hart`'s program.
    #[must_use]
    pub fn get(&self, hart: HartId) -> Option<&Process> {
        self.slots.get(hart.as_index())?.as_ref().map(|slot| &slot.process)
    }

    /// `hart`'s program, to change it.
    pub fn get_mut(&mut self, hart: HartId) -> Option<&mut Process> {
        self.slots.get_mut(hart.as_index())?.as_mut().map(|slot| &mut slot.process)
    }

    /// `hart`'s program together with the frames it maps from.
    pub fn program_and_frames(
        &mut self,
        hart: HartId,
    ) -> Option<(&mut Process, &mut FrameAllocator)> {
        let process = &mut self.slots.get_mut(hart.as_index())?.as_mut()?.process;
        Some((process, self.frames.as_mut()?))
    }

    /// Records that `hart`'s program exited with `code`, and says whether
    /// every program has now exited.
    pub fn record_exit(&mut self, hart: HartId, code: u64) -> bool {
        if let Some(slot) = self.slots.get_mut(hart.as_index()).and_then(Option::as_mut) {
            let _ = slot.exit_code.get_or_insert(code);
        }
        self.slots.iter().flatten().all(|slot| slot.exit_code.is_some())
    }

    /// Each hart's program's exit code: `None` for a hart without a
    /// program or whose program is still running.
    #[must_use]
    pub fn exit_codes(&self) -> Vec<Option<u64>> {
        self.slots.iter().map(|slot| slot.as_ref().and_then(|slot| slot.exit_code)).collect()
    }

    /// The run's exit code once every program has exited: the first
    /// non-zero code in hart order, else 0.
    #[must_use]
    pub fn run_exit_code(&self) -> u64 {
        self.exit_codes().into_iter().flatten().find(|&code| code != 0).unwrap_or(0)
    }

    /// How many host files the programs have open between them.
    #[must_use]
    pub fn host_files_open(&self) -> usize {
        self.slots.iter().flatten().map(|slot| slot.process.files.host_files_open()).sum()
    }

    /// The state a checkpoint keeps.
    #[must_use]
    pub fn checkpoint(&self) -> ProcessesState {
        ProcessesState {
            frames: self.frames.clone(),
            slots: self
                .slots
                .iter()
                .map(|slot| slot.as_ref().map(|slot| (slot.process.checkpoint(), slot.exit_code)))
                .collect(),
        }
    }

    /// The programs a checkpoint holds, each with only the console open.
    #[must_use]
    pub fn from_checkpoint(state: ProcessesState, terminal: &SharedTerminal) -> Self {
        Self {
            frames: state.frames,
            slots: state
                .slots
                .into_iter()
                .map(|slot| {
                    slot.map(|(process, exit_code)| Slot {
                        process: Process::from_checkpoint(process, terminal.clone()),
                        exit_code,
                    })
                })
                .collect(),
        }
    }
}
