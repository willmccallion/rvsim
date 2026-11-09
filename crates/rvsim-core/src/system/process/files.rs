//! The process's file descriptors, served by the host's filesystem as
//! gem5's SE mode serves them.
//!
//! Host files are reached through Unix's byte paths and metadata. On other
//! hosts the console and generated files still work, and the calls that
//! would reach a host file fail with `ENOSYS`.

use super::errno::Errno;
use crate::soc::terminal::SharedTerminal;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

/// The longest path a guest may pass, as Linux's `PATH_MAX`.
pub const PATH_MAX: usize = 4096;

/// Linux `open` flags on riscv64 (`asm-generic/fcntl.h`).
pub mod open_flags {
    pub const O_ACCMODE: u64 = 3;
    pub const O_RDONLY: u64 = 0;
    pub const O_WRONLY: u64 = 1;
    pub const O_RDWR: u64 = 2;
    pub const O_CREAT: u64 = 0o100;
    pub const O_EXCL: u64 = 0o200;
    pub const O_TRUNC: u64 = 0o1000;
    pub const O_APPEND: u64 = 0o2000;
    pub const O_DIRECTORY: u64 = 0o200_000;
}

/// A guest file descriptor. The kernel reads one as an `int`, so it may be
/// negative, as `AT_FDCWD` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fd(i32);

/// `openat`'s "relative to the working directory".
pub const AT_FDCWD: Fd = Fd(-100);

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a descriptor is a C int in the syscall ABI"
)]
impl Fd {
    /// The descriptor a syscall argument holds: its low 32 bits, as the
    /// kernel reads an `int`.
    #[must_use]
    pub const fn from_arg(arg: u64) -> Self {
        Self(arg as i32)
    }

    /// What the call that opened it returns.
    #[must_use]
    pub const fn return_value(self) -> u64 {
        self.0 as u64
    }

    /// Its slot in the table, which a negative descriptor has none of.
    fn slot_index(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }
}

/// Linux's `struct stat` on riscv64 (`asm-generic/stat.h`), 128 bytes.
#[derive(Debug, Clone, Copy, Default)]
#[allow(missing_docs, reason = "Linux's struct stat, field for field")]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub size: i64,
    pub blksize: i32,
    pub blocks: i64,
    pub atime: i64,
    pub atime_nsec: u64,
    pub mtime: i64,
    pub mtime_nsec: u64,
    pub ctime: i64,
    pub ctime_nsec: u64,
}

impl Stat {
    /// A read-only file of `size` bytes, as a generated file reports itself.
    #[must_use]
    pub const fn read_only_file(size: i64) -> Self {
        Self { mode: 0o100_444, nlink: 1, size, blksize: 4096, ..Self::character_device() }
    }

    /// A character device, which is what the console descriptors report.
    #[must_use]
    pub const fn character_device() -> Self {
        Self {
            dev: 0,
            ino: 0,
            mode: 0o020_666,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            size: 0,
            blksize: 1024,
            blocks: 0,
            atime: 0,
            atime_nsec: 0,
            mtime: 0,
            mtime_nsec: 0,
            ctime: 0,
            ctime_nsec: 0,
        }
    }

    /// The 128-byte guest layout.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 128] {
        let mut out = [0u8; 128];
        let mut put =
            |offset: usize, bytes: &[u8]| out[offset..offset + bytes.len()].copy_from_slice(bytes);
        put(0, &self.dev.to_le_bytes());
        put(8, &self.ino.to_le_bytes());
        put(16, &self.mode.to_le_bytes());
        put(20, &self.nlink.to_le_bytes());
        put(24, &self.uid.to_le_bytes());
        put(28, &self.gid.to_le_bytes());
        put(32, &self.rdev.to_le_bytes());
        put(48, &self.size.to_le_bytes());
        put(56, &self.blksize.to_le_bytes());
        put(64, &self.blocks.to_le_bytes());
        put(72, &self.atime.to_le_bytes());
        put(80, &self.atime_nsec.to_le_bytes());
        put(88, &self.mtime.to_le_bytes());
        put(96, &self.mtime_nsec.to_le_bytes());
        put(104, &self.ctime.to_le_bytes());
        put(112, &self.ctime_nsec.to_le_bytes());
        out
    }
}

/// What a descriptor refers to.
#[derive(Debug)]
pub enum HostFile {
    Stdin,
    Stdout,
    Stderr,
    File {
        path: PathBuf,
        file: File,
    },
    /// A read-only file whose contents the simulator made, at `position`.
    Generated {
        contents: Vec<u8>,
        position: u64,
    },
}

/// The descriptor table: 0, 1 and 2 are the simulator's console.
#[derive(Debug)]
pub struct FileTable {
    slots: Vec<Option<HostFile>>,
    terminal: SharedTerminal,
}

impl FileTable {
    /// A table with the console open as descriptors 0, 1 and 2.
    #[must_use]
    pub fn new(terminal: SharedTerminal) -> Self {
        Self {
            slots: vec![Some(HostFile::Stdin), Some(HostFile::Stdout), Some(HostFile::Stderr)],
            terminal,
        }
    }

    /// How many files, beyond the console, are open: what a checkpoint
    /// cannot carry.
    #[must_use]
    pub fn host_files_open(&self) -> usize {
        self.slots
            .iter()
            .flatten()
            .filter(|file| matches!(file, HostFile::File { .. } | HostFile::Generated { .. }))
            .count()
    }

    /// `ftruncate`.
    pub fn truncate(&mut self, fd: Fd, len: u64) -> Result<(), Errno> {
        match self.slot(fd)? {
            HostFile::File { file, .. } => file.set_len(len).map_err(Errno::from),
            _ => Err(Errno::EINVAL),
        }
    }

    fn slot(&mut self, fd: Fd) -> Result<&mut HostFile, Errno> {
        fd.slot_index()
            .and_then(|index| self.slots.get_mut(index))
            .and_then(Option::as_mut)
            .ok_or(Errno::EBADF)
    }

    /// Resolves `path` as the guest gave it, relative to the host's
    /// working directory when relative; `dirfd` other than `AT_FDCWD`
    /// resolves against that descriptor's path.
    pub fn resolve(&mut self, dirfd: Fd, path: &[u8]) -> Result<PathBuf, Errno> {
        let path = host_path(path)?;
        if path.is_absolute() || dirfd == AT_FDCWD {
            return Ok(path);
        }
        match self.slot(dirfd)? {
            HostFile::File { path: dir, .. } => Ok(dir.join(path)),
            _ => Err(Errno::ENOTDIR),
        }
    }

    /// `openat`: the new descriptor.
    pub fn open(&mut self, path: PathBuf, flags: u64, mode: u64) -> Result<Fd, Errno> {
        let access = flags & open_flags::O_ACCMODE;
        let mut options = std::fs::OpenOptions::new();
        let _ = options
            .read(access != open_flags::O_WRONLY)
            .write(access == open_flags::O_WRONLY || access == open_flags::O_RDWR)
            .create(flags & open_flags::O_CREAT != 0)
            .create_new(flags & open_flags::O_EXCL != 0 && flags & open_flags::O_CREAT != 0)
            .truncate(flags & open_flags::O_TRUNC != 0)
            .append(flags & open_flags::O_APPEND != 0);
        if flags & open_flags::O_CREAT != 0 {
            set_create_mode(&mut options, mode);
        }
        if flags & open_flags::O_DIRECTORY != 0 && !path.is_dir() {
            return Err(Errno::ENOTDIR);
        }
        let file = options.open(&path)?;
        self.install(HostFile::File { path, file })
    }

    /// Opens a read-only file holding `contents` and returns its
    /// descriptor.
    pub fn open_generated(&mut self, contents: Vec<u8>) -> Result<Fd, Errno> {
        self.install(HostFile::Generated { contents, position: 0 })
    }

    /// Puts `file` in the lowest free descriptor and returns it.
    fn install(&mut self, file: HostFile) -> Result<Fd, Errno> {
        let index = self.slots.iter().position(Option::is_none).unwrap_or(self.slots.len());
        let fd = Fd(i32::try_from(index).map_err(|_| Errno::EMFILE)?);
        if index == self.slots.len() {
            self.slots.push(None);
        }
        self.slots[index] = Some(file);
        Ok(fd)
    }

    pub fn close(&mut self, fd: Fd) -> Result<(), Errno> {
        let index = fd.slot_index().ok_or(Errno::EBADF)?;
        match self.slots.get_mut(index) {
            Some(slot @ Some(_)) => {
                *slot = None;
                Ok(())
            }
            _ => Err(Errno::EBADF),
        }
    }

    /// Reads up to `len` bytes; the console reads what the host typed.
    pub fn read(&mut self, fd: Fd, len: usize) -> Result<Vec<u8>, Errno> {
        let mut buffer = vec![0u8; len];
        let read = match self.slot(fd)? {
            HostFile::Stdin => {
                return self.terminal.lock().map(|mut t| t.read_input(len)).map_err(|_| Errno::EIO);
            }
            HostFile::Stdout | HostFile::Stderr => return Err(Errno::EBADF),
            HostFile::File { file, .. } => file.read(&mut buffer)?,
            HostFile::Generated { contents, position } => {
                let read = copy_from(contents, *position, &mut buffer);
                *position += read as u64;
                read
            }
        };
        buffer.truncate(read);
        Ok(buffer)
    }

    /// Reads up to `len` bytes at `offset` without moving the position.
    pub fn pread(&mut self, fd: Fd, len: usize, offset: u64) -> Result<Vec<u8>, Errno> {
        let mut buffer = vec![0u8; len];
        let read = match self.slot(fd)? {
            HostFile::File { file, .. } => read_at(file, &mut buffer, offset)?,
            HostFile::Generated { contents, .. } => copy_from(contents, offset, &mut buffer),
            _ => return Err(Errno::ESPIPE),
        };
        buffer.truncate(read);
        Ok(buffer)
    }

    /// Writes `bytes`; the console's descriptors both print to the console.
    pub fn write(&mut self, fd: Fd, bytes: &[u8]) -> Result<usize, Errno> {
        match self.slot(fd)? {
            HostFile::Stdin | HostFile::Generated { .. } => Err(Errno::EBADF),
            HostFile::Stdout | HostFile::Stderr => {
                self.terminal.lock().map_err(|_| Errno::EIO)?.write(bytes)?;
                Ok(bytes.len())
            }
            HostFile::File { file, .. } => file.write(bytes).map_err(Errno::from),
        }
    }

    pub fn pwrite(&mut self, fd: Fd, bytes: &[u8], offset: u64) -> Result<usize, Errno> {
        match self.slot(fd)? {
            HostFile::File { file, .. } => write_at(file, bytes, offset).map_err(Errno::from),
            HostFile::Generated { .. } => Err(Errno::EBADF),
            _ => Err(Errno::ESPIPE),
        }
    }

    /// `lseek` with Linux's `whence` (0 set, 1 current, 2 end).
    pub fn seek(&mut self, fd: Fd, offset: i64, whence: u64) -> Result<u64, Errno> {
        let target = match whence {
            0 => SeekFrom::Start(u64::try_from(offset).map_err(|_| Errno::EINVAL)?),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return Err(Errno::EINVAL),
        };
        match self.slot(fd)? {
            HostFile::File { file, .. } => file.seek(target).map_err(Errno::from),
            HostFile::Generated { contents, position } => {
                let moved = match target {
                    SeekFrom::Start(offset) => Some(offset),
                    SeekFrom::Current(offset) => position.checked_add_signed(offset),
                    SeekFrom::End(offset) => (contents.len() as u64).checked_add_signed(offset),
                };
                *position = moved.ok_or(Errno::EINVAL)?;
                Ok(*position)
            }
            _ => Err(Errno::ESPIPE),
        }
    }

    pub fn stat(&mut self, fd: Fd) -> Result<Stat, Errno> {
        match self.slot(fd)? {
            HostFile::Stdin | HostFile::Stdout | HostFile::Stderr => Ok(Stat::character_device()),
            HostFile::File { file, .. } => stat_of(&file.metadata()?),
            HostFile::Generated { contents, .. } => {
                Ok(Stat::read_only_file(i64::try_from(contents.len()).unwrap_or(i64::MAX)))
            }
        }
    }

    /// Whether `fd` is one of the console descriptors.
    pub fn is_console(&mut self, fd: Fd) -> bool {
        matches!(self.slot(fd), Ok(HostFile::Stdin | HostFile::Stdout | HostFile::Stderr))
    }
}

/// A host file's metadata as the guest sees it.
#[cfg(unix)]
#[expect(clippy::unnecessary_wraps, reason = "fails on hosts without Unix metadata")]
pub fn stat_of(meta: &std::fs::Metadata) -> Result<Stat, Errno> {
    use std::os::unix::fs::MetadataExt;
    Ok(Stat {
        dev: meta.dev(),
        ino: meta.ino(),
        mode: meta.mode(),
        nlink: u32::try_from(meta.nlink()).unwrap_or(u32::MAX),
        uid: meta.uid(),
        gid: meta.gid(),
        rdev: meta.rdev(),
        size: i64::try_from(meta.size()).unwrap_or(i64::MAX),
        blksize: i32::try_from(meta.blksize()).unwrap_or(4096),
        blocks: i64::try_from(meta.blocks()).unwrap_or(0),
        atime: meta.atime(),
        atime_nsec: u64::try_from(meta.atime_nsec()).unwrap_or(0),
        mtime: meta.mtime(),
        mtime_nsec: u64::try_from(meta.mtime_nsec()).unwrap_or(0),
        ctime: meta.ctime(),
        ctime_nsec: u64::try_from(meta.ctime_nsec()).unwrap_or(0),
    })
}

/// Host metadata is not Unix's here, so no host file has a guest `stat`.
#[cfg(not(unix))]
pub const fn stat_of(_meta: &std::fs::Metadata) -> Result<Stat, Errno> {
    Err(Errno::ENOSYS)
}

/// A path the guest gave, as the host names it: the same bytes.
#[cfg(unix)]
#[expect(clippy::unnecessary_wraps, reason = "fails on hosts without byte paths")]
pub fn host_path(bytes: &[u8]) -> Result<PathBuf, Errno> {
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(bytes)))
}

/// Host paths are not byte strings here, so no guest path names one.
#[cfg(not(unix))]
pub const fn host_path(_bytes: &[u8]) -> Result<PathBuf, Errno> {
    Err(Errno::ENOSYS)
}

#[cfg(unix)]
fn set_create_mode(options: &mut OpenOptions, mode: u64) {
    use std::os::unix::fs::OpenOptionsExt;
    let _ = options.mode(u32::try_from(mode & 0o7777).unwrap_or(0o644));
}

#[cfg(not(unix))]
const fn set_create_mode(_options: &mut OpenOptions, _mode: u64) {}

#[cfg(unix)]
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buffer, offset)
}

#[cfg(unix)]
fn write_at(file: &File, bytes: &[u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::write_at(file, bytes, offset)
}

#[cfg(not(unix))]
fn read_at(_file: &File, _buffer: &mut [u8], _offset: u64) -> io::Result<usize> {
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(not(unix))]
fn write_at(_file: &File, _bytes: &[u8], _offset: u64) -> io::Result<usize> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Copies `contents` from `offset` into `buffer`, as far as both go, and
/// returns how many bytes it copied.
fn copy_from(contents: &[u8], offset: u64, buffer: &mut [u8]) -> usize {
    let rest = usize::try_from(offset).ok().and_then(|start| contents.get(start..)).unwrap_or(&[]);
    let count = rest.len().min(buffer.len());
    buffer[..count].copy_from_slice(&rest[..count]);
    count
}

#[cfg(test)]
mod tests {
    use super::{AT_FDCWD, Fd, FileTable};
    use crate::config::Console;
    use crate::soc::terminal::Terminal;

    #[test]
    fn a_descriptor_argument_is_read_as_an_int() {
        assert_eq!(Fd::from_arg((-100_i64).cast_unsigned()), AT_FDCWD);
        assert_eq!(Fd::from_arg(1 << 32 | 1), Fd::from_arg(1));
    }

    #[cfg(unix)]
    #[test]
    fn a_path_need_not_be_utf_8() {
        use std::os::unix::ffi::OsStrExt;
        let mut files = FileTable::new(Terminal::shared(Console::Quiet));

        let path = files.resolve(AT_FDCWD, b"/data/\xff.bin");

        assert_eq!(
            path.map(|path| path.as_os_str().as_bytes().to_vec()),
            Ok(b"/data/\xff.bin".to_vec())
        );
    }
}
