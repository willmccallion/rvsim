//! The host files a program reaches with gem5's readfile and writefile
//! pseudo-operations: a script file the guest reads, and an output
//! directory it writes files into, as gem5's `--script` and `m5out`.

use std::fs::{self, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use crate::isa::reg::{REG_A0, REG_A1, REG_A2, REG_A3};
use crate::system::process::files::{errno, io_errno};
use crate::uarch::ctx::CoreCtx;

/// The longest file name writefile reads from the guest.
const NAME_MAX: usize = 4096;

/// The host files a guest reaches through pseudo-operations.
#[derive(Debug, Default)]
pub struct HostFiles {
    /// What readfile returns, in full; gem5's `System.readfile`.
    script: Option<Box<[u8]>>,
    /// Where writefile puts files; gem5's output directory.
    output_dir: Option<PathBuf>,
}

impl HostFiles {
    /// Makes `script` what readfile reads.
    pub fn set_script(&mut self, script: Vec<u8>) {
        self.script = Some(script.into_boxed_slice());
    }

    /// Makes `dir` where writefile puts files, creating it.
    ///
    /// # Errors
    ///
    /// The error creating the directory.
    pub fn set_output_dir(&mut self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        self.output_dir = Some(dir.to_path_buf());
        Ok(())
    }

    /// Up to `len` bytes of the script from `offset`: none at and past its
    /// end, and none without a script, as gem5 answers an unset readfile.
    #[must_use]
    pub fn script_bytes(&self, offset: u64, len: usize) -> &[u8] {
        let Some(script) = &self.script else { return &[] };
        let Ok(start) = usize::try_from(offset) else { return &[] };
        let start = start.min(script.len());
        &script[start..start.saturating_add(len).min(script.len())]
    }

    /// Writes `bytes` at `offset` of `name` in the output directory,
    /// creating or truncating the file at offset 0 and writing into the
    /// existing one otherwise, as gem5's writefile does, and returns how
    /// many bytes.
    ///
    /// # Errors
    ///
    /// The negated `errno`: `ENOENT` without an output directory, `EACCES`
    /// for a name that would leave it, and the host's error otherwise.
    pub fn write_output(&self, name: &str, offset: u64, bytes: &[u8]) -> Result<usize, i64> {
        let Some(dir) = &self.output_dir else { return Err(-errno::ENOENT) };
        let path = dir.join(within_directory(name).ok_or(-errno::EACCES)?);
        let mut file = if offset == 0 {
            fs::File::create(&path)
        } else {
            OpenOptions::new().read(true).write(true).open(&path)
        }
        .map_err(|e| io_errno(&e))?;
        let _ = file.seek(SeekFrom::Start(offset)).map_err(|e| io_errno(&e))?;
        file.write_all(bytes).map_err(|e| io_errno(&e))?;
        Ok(bytes.len())
    }
}

/// `name` as a path that stays inside the directory it is joined to:
/// relative, with no `..`; `None` when it would not.
fn within_directory(name: &str) -> Option<&Path> {
    let path = Path::new(name);
    let stays_inside = path.components().all(|part| matches!(part, Component::Normal(_)));
    (stays_inside && path.components().next().is_some()).then_some(path)
}

/// Performs readfile, now that every older store has been written.
///
/// The arguments are in `state`'s `a0` to `a2`: the buffer, its length
/// and the offset into the script. Returns what `a0` gets: how many bytes
/// were copied in, or `-EFAULT` when the buffer is not mapped.
pub fn read_file(state: &mut CoreCtx<'_>) -> i64 {
    let [buffer, len, offset] = [REG_A0, REG_A1, REG_A2].map(|reg| state.hart.regs.read(reg));
    let bytes = state.host_files.script_bytes(offset, len_as_usize(len)).to_vec();
    match state.write_now(buffer, &bytes) {
        Ok(()) => bytes.len() as i64,
        Err(trap) => {
            tracing::warn!(target: "rvsim::pseudo_op", pc = %crate::common::trace::Hex(state.hart.pc),
                buffer = %crate::common::trace::Hex(buffer), ?trap, "readfile: the buffer is not mapped");
            -errno::EFAULT
        }
    }
}

/// Performs writefile, now that every older store has been written.
///
/// The arguments are in `state`'s `a0` to `a3`: the buffer, its length,
/// the offset in the file and the file's name. Returns what `a0` gets: how
/// many bytes were written, or the negated `errno`.
pub fn write_file(state: &mut CoreCtx<'_>) -> i64 {
    let [buffer, len, offset, name] =
        [REG_A0, REG_A1, REG_A2, REG_A3].map(|reg| state.hart.regs.read(reg));
    let result = state
        .read_cstr_now(name, NAME_MAX)
        .and_then(|name| state.read_now(buffer, len_as_usize(len)).map(|bytes| (name, bytes)))
        .map_err(|trap| {
            tracing::warn!(target: "rvsim::pseudo_op", pc = %crate::common::trace::Hex(state.hart.pc),
                buffer = %crate::common::trace::Hex(buffer), ?trap, "writefile: the buffer or name is not mapped");
            -errno::EFAULT
        })
        .and_then(|(name, bytes)| {
            let name = String::from_utf8_lossy(&name);
            state.host_files.write_output(&name, offset, &bytes).inspect_err(|code| {
                tracing::warn!(target: "rvsim::pseudo_op", pc = %crate::common::trace::Hex(state.hart.pc),
                    name = %name, errno = -code, "writefile: the host file could not be written");
            })
        });
    match result {
        Ok(written) => written as i64,
        Err(negated_errno) => negated_errno,
    }
}

/// A guest length as a host count.
fn len_as_usize(len: u64) -> usize {
    usize::try_from(len).unwrap_or(usize::MAX)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn with_script(script: &[u8]) -> HostFiles {
        let mut files = HostFiles::default();
        files.set_script(script.to_vec());
        files
    }

    #[test]
    fn the_script_is_read_in_pieces_and_ends_with_nothing() {
        let files = with_script(b"hello world");

        assert_eq!(files.script_bytes(0, 5), b"hello");
        assert_eq!(files.script_bytes(5, 100), b" world");
        assert_eq!(files.script_bytes(11, 5), b"");
        assert_eq!(files.script_bytes(u64::MAX, 5), b"");
    }

    #[test]
    fn without_a_script_there_is_nothing_to_read() {
        assert_eq!(HostFiles::default().script_bytes(0, 16), b"");
    }

    #[test]
    fn a_file_is_created_at_offset_zero_and_extended_at_a_later_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = HostFiles::default();
        files.set_output_dir(&dir.path().join("out")).unwrap();

        assert_eq!(files.write_output("log.txt", 0, b"first "), Ok(6));
        assert_eq!(files.write_output("log.txt", 6, b"second"), Ok(6));

        assert_eq!(fs::read(dir.path().join("out/log.txt")).unwrap(), b"first second");
    }

    #[test]
    fn a_name_that_leaves_the_output_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = HostFiles::default();
        files.set_output_dir(dir.path()).unwrap();

        assert_eq!(files.write_output("../escape", 0, b"x"), Err(-errno::EACCES));
        assert_eq!(files.write_output("/etc/escape", 0, b"x"), Err(-errno::EACCES));
        assert_eq!(files.write_output("", 0, b"x"), Err(-errno::EACCES));
    }

    #[test]
    fn without_an_output_directory_nothing_is_written() {
        assert_eq!(HostFiles::default().write_output("a", 0, b"x"), Err(-errno::ENOENT));
    }
}
