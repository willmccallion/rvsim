//! Bus arbitration and device lookup tests.
//!
//! Tests in this module used the deleted `Memory` device + synchronous
//! `bus.read_u*/write_u*` API. Bus dispatch and IRQ aggregation are now
//! exercised end-to-end via the packet-based `Simulator` integration tests.
