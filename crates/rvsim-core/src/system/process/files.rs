//! The process's file descriptors, served by the host's filesystem as
//! gem5's SE mode serves them.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Linux `errno` values a syscall returns negated.
pub mod errno {
    pub const ENOENT: i64 = 2;
    pub const EBADF: i64 = 9;
    pub const ENOMEM: i64 = 12;
    pub const EACCES: i64 = 13;
    pub const EFAULT: i64 = 14;
    pub const EEXIST: i64 = 17;
    pub const ENOTDIR: i64 = 20;
    pub const EINVAL: i64 = 22;
    pub const EMFILE: i64 = 24;
    pub const ENOTTY: i64 = 25;
    pub const ESPIPE: i64 = 29;
    pub const ERANGE: i64 = 34;
    pub const ENOSYS: i64 = 38;
}

/// Linux `open` flags on riscv64 (`asm-generic/fcntl.h`).
pub mod open_flags {
    pub const O_ACCMODE: u64 = 3;
    pub const O_WRONLY: u64 = 1;
    pub const O_RDWR: u64 = 2;
    pub const O_CREAT: u64 = 0o100;
    pub const O_EXCL: u64 = 0o200;
    pub const O_TRUNC: u64 = 0o1000;
    pub const O_APPEND: u64 = 0o2000;
    pub const O_DIRECTORY: u64 = 0o200_000;
}

/// `openat`'s "relative to the working directory".
pub const AT_FDCWD: i64 = -100;

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
    /// The host file's metadata as the guest sees it.
    #[must_use]
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        Self {
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
        }
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
    File { path: PathBuf, file: File },
}

/// The descriptor table: 0, 1 and 2 are the host's console.
#[derive(Debug)]
pub struct FileTable {
    slots: Vec<Option<HostFile>>,
}

impl Default for FileTable {
    fn default() -> Self {
        Self { slots: vec![Some(HostFile::Stdin), Some(HostFile::Stdout), Some(HostFile::Stderr)] }
    }
}

impl FileTable {
    /// A table whose descriptor 2 is the host's stdout, as fesvr's proxy
    /// maps the target's stderr.
    #[must_use]
    pub fn with_stderr_on_stdout() -> Self {
        Self { slots: vec![Some(HostFile::Stdin), Some(HostFile::Stdout), Some(HostFile::Stdout)] }
    }

    /// How many host files, beyond the console, are open.
    #[must_use]
    pub fn host_files_open(&self) -> usize {
        self.slots.iter().flatten().filter(|file| matches!(file, HostFile::File { .. })).count()
    }

    /// `ftruncate`.
    pub fn truncate(&mut self, fd: i64, len: u64) -> Result<(), i64> {
        match self.slot(fd)? {
            HostFile::File { file, .. } => file.set_len(len).map_err(|e| io_errno(&e)),
            _ => Err(-errno::EINVAL),
        }
    }

    fn slot(&mut self, fd: i64) -> Result<&mut HostFile, i64> {
        usize::try_from(fd)
            .ok()
            .and_then(|index| self.slots.get_mut(index))
            .and_then(Option::as_mut)
            .ok_or(-errno::EBADF)
    }

    /// Resolves `path` as the guest gave it, relative to the host's
    /// working directory when relative; `dirfd` other than `AT_FDCWD`
    /// resolves against that descriptor's path.
    pub fn resolve(&mut self, dirfd: i64, path: &[u8]) -> Result<PathBuf, i64> {
        let path = Path::new(std::str::from_utf8(path).map_err(|_| -errno::EINVAL)?);
        if path.is_absolute() || dirfd == AT_FDCWD {
            return Ok(path.to_path_buf());
        }
        match self.slot(dirfd)? {
            HostFile::File { path: dir, .. } => Ok(dir.join(path)),
            _ => Err(-errno::ENOTDIR),
        }
    }

    /// `openat`: the new descriptor, or `-errno`.
    pub fn open(&mut self, path: PathBuf, flags: u64, mode: u64) -> Result<i64, i64> {
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
            use std::os::unix::fs::OpenOptionsExt;
            let _ = options.mode(u32::try_from(mode & 0o7777).unwrap_or(0o644));
        }
        if flags & open_flags::O_DIRECTORY != 0 && !path.is_dir() {
            return Err(-errno::ENOTDIR);
        }
        let file = options.open(&path).map_err(|e| io_errno(&e))?;
        let fd = self.slots.iter().position(Option::is_none).unwrap_or_else(|| {
            self.slots.push(None);
            self.slots.len() - 1
        });
        self.slots[fd] = Some(HostFile::File { path, file });
        i64::try_from(fd).map_err(|_| -errno::EMFILE)
    }

    pub fn close(&mut self, fd: i64) -> Result<(), i64> {
        let index = usize::try_from(fd).map_err(|_| -errno::EBADF)?;
        match self.slots.get_mut(index) {
            Some(slot @ Some(_)) => {
                *slot = None;
                Ok(())
            }
            _ => Err(-errno::EBADF),
        }
    }

    /// Reads up to `len` bytes; the console's stdin reads from the host's.
    pub fn read(&mut self, fd: i64, len: usize) -> Result<Vec<u8>, i64> {
        let mut buffer = vec![0u8; len];
        let read = match self.slot(fd)? {
            HostFile::Stdin => io::stdin().read(&mut buffer).map_err(|e| io_errno(&e))?,
            HostFile::Stdout | HostFile::Stderr => return Err(-errno::EBADF),
            HostFile::File { file, .. } => file.read(&mut buffer).map_err(|e| io_errno(&e))?,
        };
        buffer.truncate(read);
        Ok(buffer)
    }

    /// Reads up to `len` bytes at `offset` without moving the position.
    pub fn pread(&mut self, fd: i64, len: usize, offset: u64) -> Result<Vec<u8>, i64> {
        use std::os::unix::fs::FileExt;
        let mut buffer = vec![0u8; len];
        let read = match self.slot(fd)? {
            HostFile::File { file, .. } => {
                file.read_at(&mut buffer, offset).map_err(|e| io_errno(&e))?
            }
            _ => return Err(-errno::ESPIPE),
        };
        buffer.truncate(read);
        Ok(buffer)
    }

    pub fn write(&mut self, fd: i64, bytes: &[u8]) -> Result<usize, i64> {
        match self.slot(fd)? {
            HostFile::Stdin => Err(-errno::EBADF),
            HostFile::Stdout => {
                let mut out = io::stdout().lock();
                out.write_all(bytes).and_then(|()| out.flush()).map_err(|e| io_errno(&e))?;
                Ok(bytes.len())
            }
            HostFile::Stderr => {
                let mut err = io::stderr().lock();
                err.write_all(bytes).and_then(|()| err.flush()).map_err(|e| io_errno(&e))?;
                Ok(bytes.len())
            }
            HostFile::File { file, .. } => file.write(bytes).map_err(|e| io_errno(&e)),
        }
    }

    pub fn pwrite(&mut self, fd: i64, bytes: &[u8], offset: u64) -> Result<usize, i64> {
        use std::os::unix::fs::FileExt;
        match self.slot(fd)? {
            HostFile::File { file, .. } => file.write_at(bytes, offset).map_err(|e| io_errno(&e)),
            _ => Err(-errno::ESPIPE),
        }
    }

    /// `lseek` with Linux's `whence` (0 set, 1 current, 2 end).
    pub fn seek(&mut self, fd: i64, offset: i64, whence: u64) -> Result<u64, i64> {
        let target = match whence {
            0 => SeekFrom::Start(u64::try_from(offset).map_err(|_| -errno::EINVAL)?),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return Err(-errno::EINVAL),
        };
        match self.slot(fd)? {
            HostFile::File { file, .. } => file.seek(target).map_err(|e| io_errno(&e)),
            _ => Err(-errno::ESPIPE),
        }
    }

    pub fn stat(&mut self, fd: i64) -> Result<Stat, i64> {
        match self.slot(fd)? {
            HostFile::Stdin | HostFile::Stdout | HostFile::Stderr => Ok(Stat::character_device()),
            HostFile::File { file, .. } => {
                file.metadata().map(|meta| Stat::from_metadata(&meta)).map_err(|e| io_errno(&e))
            }
        }
    }

    /// Whether `fd` is one of the console descriptors.
    pub fn is_console(&mut self, fd: i64) -> bool {
        matches!(self.slot(fd), Ok(HostFile::Stdin | HostFile::Stdout | HostFile::Stderr))
    }
}

/// A host I/O error as the guest's negated `errno`.
pub fn io_errno(error: &io::Error) -> i64 {
    error.raw_os_error().map_or_else(
        || match error.kind() {
            io::ErrorKind::NotFound => -errno::ENOENT,
            io::ErrorKind::PermissionDenied => -errno::EACCES,
            io::ErrorKind::AlreadyExists => -errno::EEXIST,
            _ => -errno::EINVAL,
        },
        |code| -i64::from(code),
    )
}
