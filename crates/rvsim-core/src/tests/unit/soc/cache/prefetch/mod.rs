//! Prefetcher tests.

use crate::common::VirtAddr;
use crate::soc::cache::prefetch::{Lookup, Prefetcher};

pub mod next_line;
pub mod store_stream;
pub mod stream;
pub mod stride;
pub mod tagged;

/// The addresses `prefetcher` asks for after an access to `addr`.
fn observe(
    prefetcher: &mut impl Prefetcher,
    addr: u64,
    pc: Option<VirtAddr>,
    lookup: Lookup,
) -> Vec<u64> {
    let mut prefetches = Vec::new();
    prefetcher.observe(addr, pc, lookup, &mut prefetches);
    prefetches
}
