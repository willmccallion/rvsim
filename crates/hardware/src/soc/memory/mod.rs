//! Physical System Memory (DRAM): backing buffer, mapping device, and latency controller.

/// Physical-address ↔ DRAM-coordinate mapping.
pub mod address;

/// DRAM buffer implementation (e.g., mmap or `Vec`) for raw byte storage.
pub mod buffer;

/// Memory controller implementations for access latency modeling.
pub mod controller;

/// DDR5 controller with per-bank command state machines and JEDEC timing.
pub mod ddr5;
