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

/// First-In, First-Out replacement policy.
pub mod fifo;

/// Least Recently Used replacement policy.
pub mod lru;

/// Most Recently Used replacement policy.
pub mod mru;

/// Pseudo-LRU (tree-based) replacement policy.
pub mod plru;

/// Random replacement policy.
pub mod random;

pub use fifo::FifoPolicy;
pub use lru::LruPolicy;
pub use mru::MruPolicy;
pub use plru::PlruPolicy;
pub use random::RandomPolicy;

/// Trait for cache replacement policies.
///
/// Defines the interface for updating usage state and selecting victim lines.
pub trait ReplacementPolicy: Send + Sync {
    /// Updates the policy state when a line is accessed.
    ///
    /// # Arguments
    ///
    /// * `set` - The cache set index.
    /// * `way` - The way index within the set that was accessed.
    fn update(&mut self, set: usize, way: usize);

    /// Selects a victim line to evict from a specific set.
    ///
    /// # Arguments
    ///
    /// * `set` - The cache set index.
    ///
    /// # Returns
    ///
    /// The index of the way to evict.
    fn get_victim(&mut self, set: usize) -> usize;
}
