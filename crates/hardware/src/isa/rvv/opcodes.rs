//! RVV opcode constants.

/// Vector arithmetic instructions (OP-V).
pub const OP_V: u32 = 0b1010111;
// Vector loads reuse OP_LOAD_FP  = 0b0000111 (from rv64f)
// Vector stores reuse OP_STORE_FP = 0b0100111 (from rv64f)
