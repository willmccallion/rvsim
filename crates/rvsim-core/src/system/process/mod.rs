//! SE mode: Linux user programs run without a kernel.
//!
//! The simulator loads each static ELF, builds its page tables and stack,
//! and runs it in user mode on a hart; an `ecall` retires as an
//! instruction whose effect is the syscall, served by the simulator
//! against the host's files, clock and memory, as gem5's SE mode does.
//! Syscalls cost the pipeline a flush and nothing else; there is no
//! kernel to charge for. Each program is a process whose threads hold a
//! hart each ([`Processes`]); the processes share RAM's frames and the
//! caches below their cores, and the run ends when the last of them exits.

pub(crate) mod files;
pub mod loader;
mod page_table;
pub(crate) mod syscall;
mod threads;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::VirtAddr;
use crate::isa::privileged::PagingMode;
use crate::sim::memory::GlobalMemory;
use crate::soc::terminal::SharedTerminal;

pub use page_table::{AddressSpace, FrameAllocator, PAGE_SIZE, Permissions, SpaceError};
pub use threads::{Processes, ProcessesState, ThreadRef, Tid};

use files::FileTable;

/// What a process's threads share: its address space, descriptors,
/// heap and mappings, and its clock.
#[derive(Debug)]
pub struct Process {
    /// Its page tables and mappings.
    pub space: AddressSpace,
    /// Its descriptors.
    pub(crate) files: FileTable,
    /// Where `brk` started: the end of the loaded image, page-aligned.
    brk_start: VirtAddr,
    /// The current program break.
    brk: VirtAddr,
    /// Where the next anonymous `mmap` without an address goes.
    mmap_next: VirtAddr,
    /// The core clock the time syscalls divide cycles by.
    pub clock_hz: u64,
    /// Deterministic state for `getrandom`.
    random: u64,
    /// `mmap`ed file ranges, so `munmap` can forget them: start to length.
    file_maps: BTreeMap<VirtAddr, u64>,
    /// How many syscalls were served, by number.
    pub syscall_counts: BTreeMap<u64, u64>,
}

/// A process's state in a checkpoint: everything but its host files,
/// which a restored run does not get back.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessState {
    space: AddressSpace,
    brk_start: VirtAddr,
    brk: VirtAddr,
    mmap_next: VirtAddr,
    clock_hz: u64,
    random: u64,
    file_maps: BTreeMap<VirtAddr, u64>,
    syscall_counts: BTreeMap<u64, u64>,
}

/// Where a process's regions start, relative to the address space's top.
struct Layout {
    stack_top: VirtAddr,
    stack_bytes: u64,
    mmap_base: VirtAddr,
}

impl Layout {
    const STACK_BYTES: u64 = 8 << 20;

    /// Linux's layout for the mode: the stack at the top of user space,
    /// anonymous mappings from the middle up, the heap after the image.
    const fn for_space(space: &AddressSpace) -> Self {
        let top = space.user_top();
        let mmap_base = VirtAddr::new(top.val() / 2);
        Self { stack_top: top, stack_bytes: Self::STACK_BYTES, mmap_base }
    }
}

impl Process {
    /// A process over an address space whose heap starts at `brk_start`.
    #[must_use]
    pub fn new(
        space: AddressSpace,
        brk_start: VirtAddr,
        clock_hz: u64,
        terminal: SharedTerminal,
    ) -> Self {
        let layout = Layout::for_space(&space);
        Self {
            space,
            files: FileTable::new(terminal),
            brk_start,
            brk: brk_start,
            mmap_next: layout.mmap_base,
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
            space: self.space.clone(),
            brk_start: self.brk_start,
            brk: self.brk,
            mmap_next: self.mmap_next,
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
            space: state.space,
            files: FileTable::new(terminal),
            brk_start: state.brk_start,
            brk: state.brk,
            mmap_next: state.mmap_next,
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
    pub const fn brk(&self) -> VirtAddr {
        self.brk
    }

    /// Moves the program break to `new_brk`, mapping or unmapping the
    /// pages between, and returns where it ends up: unchanged when the
    /// request is below the start or cannot be mapped, as Linux does.
    pub fn set_brk(
        &mut self,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
        new_brk: VirtAddr,
    ) -> VirtAddr {
        if new_brk < self.brk_start || !self.space.in_range(new_brk) {
            return self.brk;
        }
        let old_end = self.brk.page_ceil();
        let new_end = new_brk.page_ceil();
        if new_end > old_end {
            let grown = new_end.val() - old_end.val();
            if self.space.map_range(memory, frames, old_end, grown, Permissions::RW).is_err() {
                return self.brk;
            }
        } else if new_end < old_end {
            self.space.unmap_range(memory, new_end, old_end.val() - new_end.val());
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
        hint: Option<VirtAddr>,
        len: u64,
        permissions: Permissions,
    ) -> Result<VirtAddr, SpaceError> {
        let len = page_ceil(len);
        let start = match hint {
            Some(address) if address.is_page_aligned() && !self.space.is_mapped(address, len) => {
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
    pub fn unmap(&mut self, memory: &mut GlobalMemory, start: VirtAddr, len: u64) {
        self.space.unmap_range(memory, start, len);
        let _ = self.file_maps.remove(&start);
    }

    /// Records a file-backed mapping so `munmap` can forget it.
    pub fn note_file_map(&mut self, start: VirtAddr, len: u64) {
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
    pub const fn stack_top(&self) -> VirtAddr {
        Layout::for_space(&self.space).stack_top
    }

    /// How much stack the loader maps below the top.
    #[must_use]
    pub const fn stack_bytes(&self) -> u64 {
        Layout::for_space(&self.space).stack_bytes
    }
}

/// A length rounded up to a whole number of pages.
#[must_use]
pub const fn page_ceil(value: u64) -> u64 {
    (value + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}
