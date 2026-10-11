//! The syscalls about the calling thread and its process as such, rather
//! than the process's memory, files and clock: its ids and its exit
//! protocol's pointer.

use super::{Result, SYS_GETPID, SYS_GETTID, SYS_SET_TID_ADDRESS, ok};
use crate::system::process::{Processes, ThreadRef};

/// Serves syscall `number` for `caller` when it is one of these; `None`
/// for any other.
pub(super) fn dispatch(
    processes: &mut Processes,
    caller: ThreadRef,
    number: u64,
    a: [u64; 6],
) -> Option<Result> {
    let result = match number {
        SYS_GETPID => ok(caller.tgid.as_u64()),
        SYS_GETTID => ok(caller.tid.as_u64()),
        SYS_SET_TID_ADDRESS => {
            processes.set_clear_child_tid(caller, a[0]);
            ok(caller.tid.as_u64())
        }
        _ => return None,
    };
    Some(result)
}
