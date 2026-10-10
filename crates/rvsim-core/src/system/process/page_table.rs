//! A user process's address space: the page tables the simulator builds
//! in RAM for the MMU to walk, and the map of what it has mapped where,
//! which the syscall layer reads and writes user memory through.

use std::collections::BTreeMap;

use crate::common::{PAGE_SHIFT, PhysAddr};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    OutOfRange(u64),
    /// The address is not mapped.
    Unmapped(u64),
}

/// Hands out 4 KiB frames of RAM, never reusing one.
#[derive(Debug, Clone)]
struct FrameAllocator {
    next: u64,
    end: u64,
}

impl FrameAllocator {
    fn alloc_zeroed(&mut self, memory: &mut GlobalMemory) -> Result<u64, SpaceError> {
        if self.next + PAGE_SIZE > self.end {
            return Err(SpaceError::OutOfFrames);
        }
        let frame = self.next;
        self.next += PAGE_SIZE;
        memory.load(PhysAddr::new(frame), &[0u8; PAGE_SIZE as usize]);
        Ok(frame)
    }
}

/// A page the process has mapped.
#[derive(Debug, Clone, Copy)]
struct Mapping {
    frame: u64,
    permissions: Permissions,
}

/// The process's page tables and the record of its mappings.
#[derive(Debug, Clone)]
pub struct AddressSpace {
    mode: PagingMode,
    root: u64,
    frames: FrameAllocator,
    /// Virtual page number to its frame.
    pages: BTreeMap<u64, Mapping>,
}

impl AddressSpace {
    /// An empty address space whose tables and pages come from the RAM
    /// between `first_free` and `ram_end`.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn new(
        mode: PagingMode,
        first_free: u64,
        ram_end: u64,
        memory: &mut GlobalMemory,
    ) -> Result<Self, SpaceError> {
        let mut frames =
            FrameAllocator { next: (first_free + PAGE_SIZE - 1) & !(PAGE_SIZE - 1), end: ram_end };
        let root = frames.alloc_zeroed(memory)?;
        Ok(Self { mode, root, frames, pages: BTreeMap::new() })
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
        (mode << csr::SATP_MODE_SHIFT) | (self.root >> PAGE_SHIFT)
    }

    /// One past the highest virtual address a user program may use: the
    /// lower half of the mode's address space, as Linux lays it out.
    #[must_use]
    pub const fn user_top(&self) -> u64 {
        1 << (self.va_bits() - 1)
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
    pub const fn in_range(&self, vaddr: u64) -> bool {
        vaddr < self.user_top()
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
        vaddr: u64,
        permissions: Permissions,
    ) -> Result<(), SpaceError> {
        if !self.in_range(vaddr) {
            return Err(SpaceError::OutOfRange(vaddr));
        }
        let vpn = vaddr >> PAGE_SHIFT;
        let frame = match self.pages.get(&vpn) {
            Some(mapping) => mapping.frame,
            None => self.frames.alloc_zeroed(memory)?,
        };
        self.write_leaf(memory, vpn, frame, permissions)?;
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
        start: u64,
        len: u64,
        permissions: Permissions,
    ) -> Result<(), SpaceError> {
        let first = start & !(PAGE_SIZE - 1);
        let end = start.checked_add(len).ok_or(SpaceError::OutOfRange(start))?;
        let mut page = first;
        while page < end {
            self.map(memory, page, permissions)?;
            page += PAGE_SIZE;
        }
        Ok(())
    }

    /// Drops the mappings of the pages of `[start, start + len)`; the
    /// frames are not reused.
    pub fn unmap_range(&mut self, memory: &mut GlobalMemory, start: u64, len: u64) {
        let first = start & !(PAGE_SIZE - 1);
        let end = start.saturating_add(len);
        let mut page = first;
        while page < end {
            let vpn = page >> PAGE_SHIFT;
            if self.pages.remove(&vpn).is_some() {
                let _ = self.write_pte(memory, vpn, 0);
            }
            page += PAGE_SIZE;
        }
    }

    /// Whether every page of `[start, start + len)` is mapped.
    #[must_use]
    pub fn is_mapped(&self, start: u64, len: u64) -> bool {
        let first = start & !(PAGE_SIZE - 1);
        let end = start.saturating_add(len);
        let mut page = first;
        while page < end {
            if !self.pages.contains_key(&(page >> PAGE_SHIFT)) {
                return false;
            }
            page += PAGE_SIZE;
        }
        true
    }

    /// The rights of the page holding `vaddr`, when it is mapped.
    #[must_use]
    pub fn permissions(&self, vaddr: u64) -> Option<Permissions> {
        self.pages.get(&(vaddr >> PAGE_SHIFT)).map(|mapping| mapping.permissions)
    }

    /// The physical address `vaddr` maps to.
    #[must_use]
    pub fn translate(&self, vaddr: u64) -> Option<PhysAddr> {
        let mapping = self.pages.get(&(vaddr >> PAGE_SHIFT))?;
        Some(PhysAddr::new(mapping.frame | (vaddr & (PAGE_SIZE - 1))))
    }

    /// Reads `len` bytes of user memory at `vaddr`.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn read(
        &self,
        memory: &GlobalMemory,
        vaddr: u64,
        len: usize,
    ) -> Result<Vec<u8>, SpaceError> {
        let mut out = Vec::with_capacity(len);
        let mut cursor = vaddr;
        let mut left = len;
        while left > 0 {
            let paddr = self.translate(cursor).ok_or(SpaceError::Unmapped(cursor))?;
            let in_page = (PAGE_SIZE - (cursor & (PAGE_SIZE - 1))) as usize;
            let chunk = left.min(in_page);
            let bytes = memory.read_bytes(paddr, chunk).ok_or(SpaceError::Unmapped(cursor))?;
            out.extend_from_slice(&bytes);
            cursor += chunk as u64;
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
        vaddr: u64,
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
            cursor += 1;
        }
        Ok(out)
    }

    /// Writes `bytes` to user memory at `vaddr`.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn write(
        &self,
        memory: &mut GlobalMemory,
        vaddr: u64,
        bytes: &[u8],
    ) -> Result<(), SpaceError> {
        let mut cursor = vaddr;
        let mut rest = bytes;
        while !rest.is_empty() {
            let paddr = self.translate(cursor).ok_or(SpaceError::Unmapped(cursor))?;
            let in_page = (PAGE_SIZE - (cursor & (PAGE_SIZE - 1))) as usize;
            let (chunk, tail) = rest.split_at(rest.len().min(in_page));
            memory.write_bytes(Writer::External, paddr, chunk);
            cursor += chunk.len() as u64;
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
        vaddr: u64,
        value: u64,
    ) -> Result<(), SpaceError> {
        self.write(memory, vaddr, &value.to_le_bytes())
    }

    /// Reads a little-endian `u64` from user memory.
    ///
    /// # Errors
    ///
    /// [`SpaceError`] when the address is unmapped or out of range, or RAM is full.
    pub fn read_u64(&self, memory: &GlobalMemory, vaddr: u64) -> Result<u64, SpaceError> {
        let bytes = self.read(memory, vaddr, 8)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(word))
    }

    fn write_leaf(
        &mut self,
        memory: &mut GlobalMemory,
        vpn: u64,
        frame: u64,
        permissions: Permissions,
    ) -> Result<(), SpaceError> {
        let pte = (frame >> PAGE_SHIFT) << PTE_PPN_SHIFT | permissions.pte_bits();
        self.write_pte(memory, vpn, pte)
    }

    /// Writes the leaf PTE for `vpn`, creating the tables on the way.
    fn write_pte(
        &mut self,
        memory: &mut GlobalMemory,
        vpn: u64,
        pte: u64,
    ) -> Result<(), SpaceError> {
        let mut table = self.root;
        let levels = self.levels();
        for level in (1..levels).rev() {
            let index = (vpn >> (9 * level)) & (ENTRIES_PER_TABLE - 1);
            let slot = PhysAddr::new(table + index * PTE_SIZE);
            let entry = memory.read(slot, 8).unwrap_or(0);
            table = if entry & PTE_VALID != 0 {
                (entry >> PTE_PPN_SHIFT) << PAGE_SHIFT
            } else {
                if pte == 0 {
                    return Ok(());
                }
                let next = self.frames.alloc_zeroed(memory)?;
                let pointer = (next >> PAGE_SHIFT) << PTE_PPN_SHIFT | PTE_VALID;
                memory.write(Writer::External, slot, pointer, 8);
                next
            };
        }
        let index = vpn & (ENTRIES_PER_TABLE - 1);
        memory.write(Writer::External, PhysAddr::new(table + index * PTE_SIZE), pte, 8);
        Ok(())
    }
}
