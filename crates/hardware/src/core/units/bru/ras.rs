//! Return Address Stack (RAS).
//!
//! The RAS is a specialized predictor for function return addresses. It operates
//! as a hardware stack that pushes addresses on function calls and pops them
//! on returns to predict the execution flow.

/// Return Address Stack structure.
pub struct Ras {
    /// The stack storage.
    stack: Vec<u64>,
    /// Current stack pointer index.
    ptr: usize,
    /// Maximum capacity of the stack.
    capacity: usize,
}

impl Ras {
    /// Creates a new Return Address Stack with the specified capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            stack: vec![0; capacity],
            ptr: 0,
            capacity,
        }
    }

    /// Pushes a return address onto the stack.
    ///
    /// If the stack is full, the last entry is overwritten to maintain the
    /// most recent call history.
    ///
    /// # Arguments
    ///
    /// * `addr` - The return address to push.
    pub fn push(&mut self, addr: u64) {
        if self.ptr < self.capacity {
            self.stack[self.ptr] = addr;
            self.ptr += 1;
        } else {
            self.stack[self.capacity - 1] = addr;
        }
    }

    /// Pops a return address from the stack.
    ///
    /// # Returns
    ///
    /// The popped return address, or `None` if the stack is empty.
    pub fn pop(&mut self) -> Option<u64> {
        if self.ptr == 0 {
            None
        } else {
            self.ptr -= 1;
            Some(self.stack[self.ptr])
        }
    }

    /// Peeks at the top of the stack without removing the entry.
    ///
    /// Used to predict the target of a return instruction without modifying
    /// the stack state until the instruction is committed.
    ///
    /// # Returns
    ///
    /// The return address at the top of the stack, or `None` if empty.
    pub fn top(&self) -> Option<u64> {
        if self.ptr == 0 {
            None
        } else {
            Some(self.stack[self.ptr - 1])
        }
    }
}
