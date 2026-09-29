//! Register renaming structures.

/// Checkpoint table for O(1) branch misprediction recovery.
pub mod checkpoint;

/// Physical register free list.
pub mod free_list;

/// Speculative rename map (arch reg → physical reg).
pub mod map;

/// Physical register file with ready bits.
pub mod prf;

/// Tag-based register scoreboard.
pub mod scoreboard;

/// Vector physical register file with ready bits and `VecPrfView`.
pub mod vec_prf;
