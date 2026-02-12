//! Instruction pipeline implementation.
//!
//! This module contains the implementation of the five-stage instruction pipeline.
//! It includes the following components:
//! 1. **Hazards:** Detection and resolution of data and structural hazards.
//! 2. **Latches:** Inter-stage buffers for communication between pipeline stages.
//! 3. **Signals:** Control signals generated during instruction decoding.
//! 4. **Stages:** Implementation of Fetch, Decode, Execute, Memory, and Writeback stages.
//! 5. **Traits:** Common interfaces for pipeline components and stages.

/// Pipeline hazard detection and forwarding logic.
pub mod hazards;

/// Inter-stage pipeline latches (IF/ID, ID/EX, EX/MEM, MEM/WB).
pub mod latches;

/// Control signals generated during instruction decode.
pub mod signals;

/// Pipeline stage implementations (fetch, decode, execute, memory, writeback).
pub mod stages;

/// Traits for pipeline stage components.
pub mod traits;
