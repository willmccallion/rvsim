//! The file syscalls HTIF's proxy and SE mode both serve, written once over
//! the memory each finds the guest's buffers in.

use super::AddressSpace;
use super::errno::Errno;
use super::files::{Fd, FileTable, Stat};
use crate::common::{PhysAddr, VirtAddr};
use crate::sim::memory::GlobalMemory;
use crate::sim::memory::write_log::Writer;
use std::path::Path;

/// `unlinkat`'s flag to remove a directory instead of a file.
const AT_REMOVEDIR: u64 = 0x200;

/// Where a syscall's buffers are.
pub trait GuestMemory {
    /// A guest address as the syscall passes it.
    type Addr: Copy;

    /// The `len` bytes at `addr`.
    fn read(&self, addr: Self::Addr, len: usize) -> Result<Vec<u8>, Errno>;

    /// Writes `bytes` at `addr`.
    fn write(&mut self, addr: Self::Addr, bytes: &[u8]) -> Result<(), Errno>;
}

/// Physical memory, where HTIF's target passes its buffers.
#[derive(Debug)]
pub struct PhysicalMemory<'a>(pub &'a mut GlobalMemory);

impl GuestMemory for PhysicalMemory<'_> {
    type Addr = PhysAddr;

    fn read(&self, addr: PhysAddr, len: usize) -> Result<Vec<u8>, Errno> {
        self.0.read_bytes(addr, len).map(Vec::from).ok_or(Errno::EFAULT)
    }

    fn write(&mut self, addr: PhysAddr, bytes: &[u8]) -> Result<(), Errno> {
        if !bytes.is_empty() {
            self.0.write_bytes(Writer::External, addr, bytes);
        }
        Ok(())
    }
}

/// A process's address space, where an SE-mode program passes its buffers.
#[derive(Debug)]
pub struct UserMemory<'a> {
    /// The process's mappings.
    pub space: &'a AddressSpace,
    /// The RAM they map.
    pub memory: &'a mut GlobalMemory,
}

impl GuestMemory for UserMemory<'_> {
    type Addr = VirtAddr;

    fn read(&self, addr: VirtAddr, len: usize) -> Result<Vec<u8>, Errno> {
        Ok(self.space.read(self.memory, addr, len)?)
    }

    fn write(&mut self, addr: VirtAddr, bytes: &[u8]) -> Result<(), Errno> {
        Ok(self.space.write(self.memory, addr, bytes)?)
    }
}

/// The guest's descriptors and memory, for the calls that move bytes
/// between them. Each returns what the guest's `a0` gets on success.
#[derive(Debug)]
pub struct FileCalls<'a, M> {
    /// The descriptors.
    pub files: &'a mut FileTable,
    /// Where the buffers are.
    pub memory: M,
}

impl<M: GuestMemory> FileCalls<'_, M> {
    /// `write`: the `len` bytes at `buf`.
    pub fn write(&mut self, fd: Fd, buf: M::Addr, len: u64) -> Result<u64, Errno> {
        let bytes = self.memory.read(buf, buffer_len(len)?)?;
        Ok(self.files.write(fd, &bytes)? as u64)
    }

    /// `read`: up to `len` bytes into `buf`.
    pub fn read(&mut self, fd: Fd, buf: M::Addr, len: u64) -> Result<u64, Errno> {
        let bytes = self.files.read(fd, buffer_len(len)?)?;
        self.memory.write(buf, &bytes)?;
        Ok(bytes.len() as u64)
    }

    /// `pread64`: up to `len` bytes from `offset` into `buf`.
    pub fn pread(&mut self, fd: Fd, buf: M::Addr, len: u64, offset: u64) -> Result<u64, Errno> {
        let bytes = self.files.pread(fd, buffer_len(len)?, offset)?;
        self.memory.write(buf, &bytes)?;
        Ok(bytes.len() as u64)
    }

    /// `pwrite64`: the `len` bytes at `buf`, at `offset`.
    pub fn pwrite(&mut self, fd: Fd, buf: M::Addr, len: u64, offset: u64) -> Result<u64, Errno> {
        let bytes = self.memory.read(buf, buffer_len(len)?)?;
        Ok(self.files.pwrite(fd, &bytes, offset)? as u64)
    }

    /// `fstat`: the descriptor's `struct stat` into `buf`.
    pub fn fstat(&mut self, fd: Fd, buf: M::Addr) -> Result<u64, Errno> {
        let stat = self.files.stat(fd)?;
        self.write_stat(stat, buf)
    }

    /// `stat` into `buf`, for the calls that stat a path.
    pub fn write_stat(&mut self, stat: Stat, buf: M::Addr) -> Result<u64, Errno> {
        self.memory.write(buf, &stat.to_bytes())?;
        Ok(0)
    }

    /// `readlinkat`'s answer: as much of the link's `target` as fits in
    /// `size` bytes, into `buf`, unterminated.
    pub fn write_link_target(
        &mut self,
        target: &[u8],
        buf: M::Addr,
        size: u64,
    ) -> Result<u64, Errno> {
        let len = target.len().min(buffer_len(size)?);
        self.memory.write(buf, &target[..len])?;
        Ok(len as u64)
    }

    /// `getcwd`: the host's working directory and its NUL into `buf`.
    pub fn getcwd(&mut self, buf: M::Addr, size: u64) -> Result<u64, Errno> {
        let mut cwd = std::env::current_dir()?.into_os_string().into_encoded_bytes();
        cwd.push(0);
        if cwd.len() as u64 > size {
            return Err(Errno::ERANGE);
        }
        self.memory.write(buf, &cwd)?;
        Ok(cwd.len() as u64)
    }
}

/// A buffer length the guest passed, which must fit in host memory.
pub fn buffer_len(len: u64) -> Result<usize, Errno> {
    usize::try_from(len).map_err(|_| Errno::EINVAL)
}

/// The `struct stat` of what `path` names, following a final link.
pub fn stat(path: &Path) -> Result<Stat, Errno> {
    Ok(Stat::from_metadata(&std::fs::metadata(path)?))
}

/// The `struct stat` of `path` itself, even when it is a link.
pub fn lstat(path: &Path) -> Result<Stat, Errno> {
    Ok(Stat::from_metadata(&std::fs::symlink_metadata(path)?))
}

/// What the link at `path` points to.
pub fn read_link(path: &Path) -> Result<Vec<u8>, Errno> {
    Ok(std::fs::read_link(path)?.into_os_string().into_encoded_bytes())
}

/// `faccessat`: whether `path` exists.
pub fn access(path: &Path) -> Result<u64, Errno> {
    std::fs::metadata(path).map(drop)?;
    Ok(0)
}

/// `mkdirat`.
pub fn make_dir(path: &Path) -> Result<u64, Errno> {
    std::fs::create_dir(path)?;
    Ok(0)
}

/// `unlinkat`: a directory with `AT_REMOVEDIR` in `flags`, a file otherwise.
pub fn unlink(path: &Path, flags: u64) -> Result<u64, Errno> {
    if flags & AT_REMOVEDIR == 0 {
        std::fs::remove_file(path)?;
    } else {
        std::fs::remove_dir(path)?;
    }
    Ok(0)
}

/// `renameat`.
pub fn rename(from: &Path, to: &Path) -> Result<u64, Errno> {
    std::fs::rename(from, to)?;
    Ok(0)
}
