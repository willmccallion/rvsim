//! Cache Replacement Policies.
//!
//! Implements various algorithms for selecting victim lines in set-associative caches.
//!
//! # Policies
//!
//! - `Fifo`: First-In, First-Out.
//! - `Lru`: Least Recently Used.
//! - `Mru`: Most Recently Used.
//! - `Plru`: Pseudo-LRU (Tree-based).
//! - `Random`: Random selection.

pub mod fifo;

pub mod lru;

pub mod mru;

pub mod plru;

pub mod random;

mod recency;

pub use fifo::FifoPolicy;
pub use lru::LruPolicy;
pub use mru::MruPolicy;
pub use plru::PlruPolicy;
pub use random::RandomPolicy;

/// Trait for cache replacement policies.
///
/// Defines the interface for updating usage state and selecting victim lines.
pub trait ReplacementPolicy: Send + Sync {
    /// Records an access to `way` of `set`.
    fn update(&mut self, set: usize, way: usize);

    /// Updates the policy state when a line is installed in `way`; for a
    /// policy that orders lines by use, the same as an access.
    fn insert(&mut self, set: usize, way: usize) {
        self.update(set, way);
    }

    /// The way of `set` to evict.
    fn get_victim(&mut self, set: usize) -> usize;
}
