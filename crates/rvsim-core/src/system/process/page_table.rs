//! A user process's address space: the page tables the simulator builds
//! in RAM for the MMU to walk, and the map of what it has mapped where,
//! which the syscall layer reads and writes user memory through.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::common::{PAGE_SHIFT, PhysAddr, VirtAddr, Vpn};
use crate::isa::csr;
use crate::isa::privileged::PagingMode;
use crate::sim::memory::GlobalMemory;
use crate::sim::memory::write_log::Writer;

/// Bytes in a page.
pub const PAGE_SIZE: u64 = 1 << PAGE_SHIFT;
/// Page-table entries in one table.
const ENTRIES_PER_TABLE: u64 = 512;
const PTE_SIZE: u64 = 8;
const PTE_VALID: u64 = 1;
const PTE_READ: u64 = 1 << 1;
const PTE_WRITE: u64 = 1 << 2;
const PTE_EXEC: u64 = 1 << 3;
const PTE_USER: u64 = 1 << 4;
const PTE_ACCESSED: u64 = 1 << 6;
const PTE_DIRTY: u64 = 1 << 7;
const PTE_PPN_SHIFT: u64 = 10;

/// What a mapped page may be used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Loads may read it.
    pub read: bool,
    /// Stores may write it.
    pub write: bool,
    /// Fetch may execute it.
    pub execute: bool,
}

impl Permissions {
    /// Readable and writable: data, stack and heap.
    pub const RW: Self = Self { read: true, write: true, execute: false };

    /// From an ELF segment's or `mmap`'s `PROT_*` bits (R=1, W=2, X=4 in
    /// both encodings).
    #[must_use]
    pub const fn from_prot(prot: u64) -> Self {
        Self { read: prot & 1 != 0, write: prot & 2 != 0, execute: prot & 4 != 0 }
    }

    /// Both sets of rights, for a page two segments share.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            read: self.read || other.read,
            write: self.write || other.write,
            execute: self.execute || other.execute,
        }
    }

    const fn pte_bits(self) -> u64 {
        let mut bits = PTE_VALID | PTE_USER | PTE_ACCESSED | PTE_DIRTY;
        if self.read {
            bits |= PTE_READ;
        }
        if self.write {
            bits |= PTE_WRITE;
        }
        if self.execute {
            bits |= PTE_EXEC;
        }
        bits
    }
}

/// Why a user memory access or mapping could not be done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceError {
    /// No RAM left for a page or a page table.
    OutOfFrames,
    /// The virtual address is above what the paging mode can name.
    OutOfRange(VirtAddr),
    /// The address is not mapped.
    Unmapped(VirtAddr),
}

/// Hands out 4 KiB frames of RAM, never reusing one; every process's
/// pages and tables come from the one allocator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameAllocator {
    next: PhysAddr,
    end: PhysAddr,
}

impl FrameAllocator {
    /// Frames from `first_free`, rounded up to a page, to `end`.
    #[must_use]
    pub const fn new(first_free: PhysAddr, end: PhysAddr) -> Self {
        let next = (first_free.val() + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        Self { next: PhysAddr::new(next), end }
    }

    fn alloc_zeroed(&mut self, memory: &mut GlobalMemory) -> Result<PhysAddr, SpaceError> {
        if self.next.val() + PAGE_SIZE > self.end.val() {
            return Err(SpaceError::OutOfFrames);
        }
        let frame = self.next;
        self.next = PhysAddr::new(frame.val() + PAGE_SIZE);
        memory.load(frame, &[0u8; PAGE_SIZE as usize]);
        Ok(frame)
    }
}

/// A page the process has mapped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Mapping {
    frame: PhysAddr,
    permissions: Permissions,
}

/// The process's page tables and the record of its mappings. The tables
/// themselves live in RAM, so a checkpoint's RAM image carries them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressSpace {
    mode: PagingMode,
    root: PhysAddr,
    pages: BTreeMap<Vpn, Mapping>,
    /// How many times a mapping was removed or had its rights changed,
    /// which a TLB may still hold the old form of.
    #[serde(skip)]
    revocations: u64,
}

impl AddressSpace {
    /// An empty address space whose tables and pages come from `frames`.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn new(
        mode: PagingMode,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
    ) -> Result<Self, SpaceError> {
        let root = frames.alloc_zeroed(memory)?;
        Ok(Self { mode, root, pages: BTreeMap::new(), revocations: 0 })
    }

    /// The paging mode the tables are built for.
    #[must_use]
    pub const fn mode(&self) -> PagingMode {
        self.mode
    }

    /// The `satp` value that selects these tables.
    #[must_use]
    pub const fn satp(&self) -> u64 {
        let mode = match self.mode {
            PagingMode::Bare => csr::SATP_MODE_BARE,
            PagingMode::Sv39 => csr::SATP_MODE_SV39,
            PagingMode::Sv48 => csr::SATP_MODE_SV48,
            PagingMode::Sv57 => csr::SATP_MODE_SV57,
        };
        (mode << csr::SATP_MODE_SHIFT) | (self.root.val() >> PAGE_SHIFT)
    }

    /// One past the highest virtual address a user program may use: the
    /// lower half of the mode's address space, as Linux lays it out.
    #[must_use]
    pub const fn user_top(&self) -> VirtAddr {
        VirtAddr::new(1 << (self.va_bits() - 1))
    }

    const fn va_bits(&self) -> u32 {
        match self.mode {
            PagingMode::Bare => 64,
            PagingMode::Sv39 => 39,
            PagingMode::Sv48 => 48,
            PagingMode::Sv57 => 57,
        }
    }

    const fn levels(&self) -> u32 {
        match self.mode {
            PagingMode::Bare => 0,
            PagingMode::Sv39 => 3,
            PagingMode::Sv48 => 4,
            PagingMode::Sv57 => 5,
        }
    }

    /// Whether `vaddr` is one a user program may use.
    #[must_use]
    pub const fn in_range(&self, vaddr: VirtAddr) -> bool {
        vaddr.val() < self.user_top().val()
    }

    /// Maps the page holding `vaddr` to a fresh zeroed frame, or changes
    /// the permissions of the page already there.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn map(
        &mut self,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
        vaddr: VirtAddr,
        permissions: Permissions,
    ) -> Result<(), SpaceError> {
        if !self.in_range(vaddr) {
            return Err(SpaceError::OutOfRange(vaddr));
        }
        let vpn = vaddr.page_number();
        let frame = match self.pages.get(&vpn) {
            Some(mapping) => {
                if mapping.permissions != permissions {
                    self.revocations += 1;
                }
                mapping.frame
            }
            None => frames.alloc_zeroed(memory)?,
        };
        self.write_leaf(memory, frames, vpn, frame, permissions)?;
        let _ = self.pages.insert(vpn, Mapping { frame, permissions });
        Ok(())
    }

    /// Maps every page of `[start, start + len)`, rounded out to pages.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn map_range(
        &mut self,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
        start: VirtAddr,
        len: u64,
        permissions: Permissions,
    ) -> Result<(), SpaceError> {
        let end = start.checked_add(len).ok_or(SpaceError::OutOfRange(start))?;
        let mut page = start.page_floor();
        while page < end {
            self.map(memory, frames, page, permissions)?;
            page = page.wrapping_add(PAGE_SIZE);
        }
        Ok(())
    }

    /// Drops the mappings of the pages of `[start, start + len)`; the
    /// frames are not reused.
    pub fn unmap_range(&mut self, memory: &mut GlobalMemory, start: VirtAddr, len: u64) {
        for vpn in Self::pages_of(start, len) {
            if self.pages.remove(&vpn).is_some() {
                self.clear_pte(memory, vpn);
                self.revocations += 1;
            }
        }
    }

    /// How many times a mapping has been removed or had its rights
    /// changed: when it moves, other harts' TLBs may hold stale entries.
    #[must_use]
    pub const fn revocations(&self) -> u64 {
        self.revocations
    }

    /// Whether every page of `[start, start + len)` is mapped.
    #[must_use]
    pub fn is_mapped(&self, start: VirtAddr, len: u64) -> bool {
        Self::pages_of(start, len).all(|vpn| self.pages.contains_key(&vpn))
    }

    /// The numbers of the pages `[start, start + len)` touches, stopping at
    /// the top of the address space.
    fn pages_of(start: VirtAddr, len: u64) -> impl Iterator<Item = Vpn> {
        let first = start.page_number().val();
        let end = start.val().saturating_add(len);
        let last = end.div_ceil(PAGE_SIZE);
        (first..last).map(Vpn::new)
    }

    /// The rights of the page holding `vaddr`, when it is mapped.
    #[must_use]
    pub fn permissions(&self, vaddr: VirtAddr) -> Option<Permissions> {
        self.pages.get(&vaddr.page_number()).map(|mapping| mapping.permissions)
    }

    /// The physical address `vaddr` maps to.
    #[must_use]
    pub fn translate(&self, vaddr: VirtAddr) -> Option<PhysAddr> {
        let mapping = self.pages.get(&vaddr.page_number())?;
        Some(PhysAddr::new(mapping.frame.val() | vaddr.page_offset()))
    }

    /// Reads `len` bytes of user memory at `vaddr`.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn read(
        &self,
        memory: &GlobalMemory,
        vaddr: VirtAddr,
        len: usize,
    ) -> Result<Vec<u8>, SpaceError> {
        let mut out = Vec::with_capacity(len);
        let mut cursor = vaddr;
        let mut left = len;
        while left > 0 {
            let paddr = self.translate(cursor).ok_or(SpaceError::Unmapped(cursor))?;
            let in_page = (PAGE_SIZE - cursor.page_offset()) as usize;
            let chunk = left.min(in_page);
            let bytes = memory.read_bytes(paddr, chunk).ok_or(SpaceError::Unmapped(cursor))?;
            out.extend_from_slice(&bytes);
            cursor = cursor.wrapping_add(chunk as u64);
            left -= chunk;
        }
        Ok(out)
    }

    /// Reads the NUL-terminated string at `vaddr`, up to `limit` bytes.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn read_cstr(
        &self,
        memory: &GlobalMemory,
        vaddr: VirtAddr,
        limit: usize,
    ) -> Result<Vec<u8>, SpaceError> {
        let mut out = Vec::new();
        let mut cursor = vaddr;
        while out.len() < limit {
            let paddr = self.translate(cursor).ok_or(SpaceError::Unmapped(cursor))?;
            let byte = memory.read(paddr, 1).ok_or(SpaceError::Unmapped(cursor))? as u8;
            if byte == 0 {
                break;
            }
            out.push(byte);
            cursor = cursor.wrapping_add(1);
        }
        Ok(out)
    }

    /// Writes `bytes` to user memory at `vaddr`, whatever the pages'
    /// rights: the kernel's view, not the program's.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn write(
        &self,
        memory: &mut GlobalMemory,
        vaddr: VirtAddr,
        bytes: &[u8],
    ) -> Result<(), SpaceError> {
        let mut cursor = vaddr;
        let mut rest = bytes;
        while !rest.is_empty() {
            let paddr = self.translate(cursor).ok_or(SpaceError::Unmapped(cursor))?;
            let in_page = (PAGE_SIZE - cursor.page_offset()) as usize;
            let (chunk, tail) = rest.split_at(rest.len().min(in_page));
            memory.write_bytes(Writer::External, paddr, chunk);
            cursor = cursor.wrapping_add(chunk.len() as u64);
            rest = tail;
        }
        Ok(())
    }

    /// Writes a little-endian `u64` to user memory.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn write_u64(
        &self,
        memory: &mut GlobalMemory,
        vaddr: VirtAddr,
        value: u64,
    ) -> Result<(), SpaceError> {
        self.write(memory, vaddr, &value.to_le_bytes())
    }

    /// Reads a little-endian `u64` from user memory.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn read_u64(&self, memory: &GlobalMemory, vaddr: VirtAddr) -> Result<u64, SpaceError> {
        let bytes = self.read(memory, vaddr, 8)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(word))
    }

    fn write_leaf(
        &self,
        memory: &mut GlobalMemory,
        frames: &mut FrameAllocator,
        vpn: Vpn,
        frame: PhysAddr,
        permissions: Permissions,
    ) -> Result<(), SpaceError> {
        let pte = Self::pointer_to(frame) | permissions.pte_bits();
        let mut table = self.root;
        for level in (1..self.levels()).rev() {
            let slot = Self::table_slot(table, vpn, level);
            let entry = memory.read(slot, 8).unwrap_or(0);
            table = if entry & PTE_VALID != 0 {
                Self::target_of(entry)
            } else {
                let next = frames.alloc_zeroed(memory)?;
                memory.write(Writer::External, slot, Self::pointer_to(next) | PTE_VALID, 8);
                next
            };
        }
        memory.write(Writer::External, Self::table_slot(table, vpn, 0), pte, 8);
        Ok(())
    }

    /// Clears the leaf PTE for `vpn`, if its tables exist.
    fn clear_pte(&self, memory: &mut GlobalMemory, vpn: Vpn) {
        let mut table = self.root;
        for level in (1..self.levels()).rev() {
            let entry = memory.read(Self::table_slot(table, vpn, level), 8).unwrap_or(0);
            if entry & PTE_VALID == 0 {
                return;
            }
            table = Self::target_of(entry);
        }
        memory.write(Writer::External, Self::table_slot(table, vpn, 0), 0, 8);
    }

    /// The PPN field of a PTE naming `frame`.
    const fn pointer_to(frame: PhysAddr) -> u64 {
        (frame.val() >> PAGE_SHIFT) << PTE_PPN_SHIFT
    }

    /// The frame a PTE names.
    const fn target_of(entry: u64) -> PhysAddr {
        PhysAddr::new((entry >> PTE_PPN_SHIFT) << PAGE_SHIFT)
    }

    /// The address of `vpn`'s entry in the table at `table` for `level`.
    const fn table_slot(table: PhysAddr, vpn: Vpn, level: u32) -> PhysAddr {
        let index = (vpn.val() >> (9 * level)) & (ENTRIES_PER_TABLE - 1);
        PhysAddr::new(table.val() + index * PTE_SIZE)
    }
}
