//! Several harts sharing memory: atomics, reservations and visibility.

pub mod amo_counter;
pub mod coherence;
pub mod idle_skip;
pub mod quiet_skip;
pub mod spinlock;
