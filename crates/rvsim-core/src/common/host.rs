//! What rvsim assumes of the host it runs on.

// Guest addresses, RAM sizes and offsets are u64 and index host memory
// directly, which only holds where usize is as wide.
const _: () = assert!(usize::BITS == u64::BITS, "rvsim needs a 64-bit host");

/// `value` as a host index or length. It never truncates: the host is
/// 64-bit, which the assertion above checks at compile time.
#[inline]
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "usize is 64 bits on every host rvsim builds for"
)]
pub const fn usize_of(value: u64) -> usize {
    value as usize
}
