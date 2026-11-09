//! Linux error numbers, which a syscall returns negated in `a0`.

use super::SpaceError;
use std::io;

/// A Linux `errno` value (`asm-generic/errno-base.h` and `errno.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Errno(u16);

#[allow(missing_docs, reason = "each is the Linux error of that name")]
impl Errno {
    pub const EPERM: Self = Self(1);
    pub const ENOENT: Self = Self(2);
    pub const ESRCH: Self = Self(3);
    pub const EINTR: Self = Self(4);
    pub const EIO: Self = Self(5);
    pub const EBADF: Self = Self(9);
    pub const EAGAIN: Self = Self(11);
    pub const ENOMEM: Self = Self(12);
    pub const EACCES: Self = Self(13);
    pub const EFAULT: Self = Self(14);
    pub const EBUSY: Self = Self(16);
    pub const EEXIST: Self = Self(17);
    pub const EXDEV: Self = Self(18);
    pub const ENOTDIR: Self = Self(20);
    pub const EISDIR: Self = Self(21);
    pub const EINVAL: Self = Self(22);
    pub const ENFILE: Self = Self(23);
    pub const EMFILE: Self = Self(24);
    pub const ENOTTY: Self = Self(25);
    pub const ETXTBSY: Self = Self(26);
    pub const EFBIG: Self = Self(27);
    pub const ENOSPC: Self = Self(28);
    pub const ESPIPE: Self = Self(29);
    pub const EROFS: Self = Self(30);
    pub const EMLINK: Self = Self(31);
    pub const EPIPE: Self = Self(32);
    pub const ERANGE: Self = Self(34);
    pub const ENAMETOOLONG: Self = Self(36);
    pub const ENOSYS: Self = Self(38);
    pub const ENOTEMPTY: Self = Self(39);
    pub const ELOOP: Self = Self(40);
    pub const EOVERFLOW: Self = Self(75);
    pub const ETIMEDOUT: Self = Self(110);
    pub const EDQUOT: Self = Self(122);

    /// What the syscall leaves in `a0`: the number, negated.
    #[must_use]
    pub const fn return_value(self) -> u64 {
        (-(self.0 as i64)).cast_unsigned()
    }

    /// The Linux number of the host's `errno` `code`. The host numbers its
    /// errors its own way (macOS's `EAGAIN` is 35, Linux's 11), so each is
    /// translated by name; one a file syscall cannot raise reads as `EIO`.
    #[cfg(unix)]
    const fn of_host(code: i32) -> Self {
        match code {
            libc::EPERM => Self::EPERM,
            libc::ENOENT => Self::ENOENT,
            libc::ESRCH => Self::ESRCH,
            libc::EINTR => Self::EINTR,
            libc::EBADF => Self::EBADF,
            libc::EAGAIN => Self::EAGAIN,
            libc::ENOMEM => Self::ENOMEM,
            libc::EACCES => Self::EACCES,
            libc::EFAULT => Self::EFAULT,
            libc::EBUSY => Self::EBUSY,
            libc::EEXIST => Self::EEXIST,
            libc::EXDEV => Self::EXDEV,
            libc::ENOTDIR => Self::ENOTDIR,
            libc::EISDIR => Self::EISDIR,
            libc::EINVAL => Self::EINVAL,
            libc::ENFILE => Self::ENFILE,
            libc::EMFILE => Self::EMFILE,
            libc::ENOTTY => Self::ENOTTY,
            libc::ETXTBSY => Self::ETXTBSY,
            libc::EFBIG => Self::EFBIG,
            libc::ENOSPC => Self::ENOSPC,
            libc::ESPIPE => Self::ESPIPE,
            libc::EROFS => Self::EROFS,
            libc::EMLINK => Self::EMLINK,
            libc::EPIPE => Self::EPIPE,
            libc::ERANGE => Self::ERANGE,
            libc::ENAMETOOLONG => Self::ENAMETOOLONG,
            libc::ENOSYS => Self::ENOSYS,
            libc::ENOTEMPTY => Self::ENOTEMPTY,
            libc::ELOOP => Self::ELOOP,
            libc::EOVERFLOW => Self::EOVERFLOW,
            libc::ETIMEDOUT => Self::ETIMEDOUT,
            libc::EDQUOT => Self::EDQUOT,
            _ => Self::EIO,
        }
    }
}

impl From<io::Error> for Errno {
    /// By the host's `errno` on Unix; elsewhere the host's error codes are
    /// not `errno`s, so by the error's kind alone.
    fn from(error: io::Error) -> Self {
        #[cfg(unix)]
        if let Some(code) = error.raw_os_error() {
            return Self::of_host(code);
        }
        match error.kind() {
            io::ErrorKind::NotFound => Self::ENOENT,
            io::ErrorKind::PermissionDenied => Self::EACCES,
            io::ErrorKind::AlreadyExists => Self::EEXIST,
            io::ErrorKind::Unsupported => Self::ENOSYS,
            _ => Self::EINVAL,
        }
    }
}

impl From<SpaceError> for Errno {
    fn from(error: SpaceError) -> Self {
        match error {
            SpaceError::OutOfFrames => Self::ENOMEM,
            SpaceError::OutOfRange(_) | SpaceError::Unmapped(_) => Self::EFAULT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Errno;
    use std::io;

    #[cfg(unix)]
    fn guest_errno(host_code: i32) -> Errno {
        Errno::from(io::Error::from_raw_os_error(host_code))
    }

    #[cfg(unix)]
    #[test]
    fn host_errors_reach_the_guest_under_their_linux_numbers() {
        assert_eq!(guest_errno(libc::EAGAIN).return_value(), (-11_i64).cast_unsigned());
        assert_eq!(guest_errno(libc::ENAMETOOLONG).return_value(), (-36_i64).cast_unsigned());
        assert_eq!(guest_errno(libc::ENOSYS).return_value(), (-38_i64).cast_unsigned());
        assert_eq!(guest_errno(libc::ENOTEMPTY).return_value(), (-39_i64).cast_unsigned());
        assert_eq!(guest_errno(libc::ELOOP).return_value(), (-40_i64).cast_unsigned());
        assert_eq!(guest_errno(libc::ETIMEDOUT).return_value(), (-110_i64).cast_unsigned());
        assert_eq!(guest_errno(libc::EDQUOT).return_value(), (-122_i64).cast_unsigned());
    }

    #[cfg(unix)]
    #[test]
    fn a_host_error_no_file_syscall_raises_reads_as_eio() {
        assert_eq!(guest_errno(libc::EHOSTUNREACH), Errno::EIO);
    }

    #[test]
    fn an_error_without_an_os_code_maps_by_kind() {
        assert_eq!(Errno::from(io::Error::new(io::ErrorKind::NotFound, "gone")), Errno::ENOENT);
    }
}
