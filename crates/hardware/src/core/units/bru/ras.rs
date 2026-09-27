//! Return Address Stack (RAS), after gem5's `ReturnAddrStack`.
//!
//! A circular stack: a push past capacity overwrites the oldest entry and a
//! pop past the bottom wraps to it. Each prediction keeps a [`RasHistory`]
//! of what it did, which a squash undoes youngest first.

/// The stack operations one prediction performed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasHistory {
    pushed: bool,
    popped: Option<PoppedEntry>,
}

/// Where the top of stack was before a pop, and what it held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoppedEntry {
    tos: usize,
    entry: Option<u64>,
}

/// Return Address Stack structure.
#[derive(Debug)]
pub struct Ras {
    /// The entries; `None` until first written.
    stack: Vec<Option<u64>>,
    /// Index of the top of stack.
    tos: usize,
}

impl Ras {
    /// Creates a Return Address Stack with `capacity` entries.
    pub fn new(capacity: usize) -> Self {
        Self { stack: vec![None; capacity], tos: 0 }
    }

    /// Pushes a call's return address, recording the push in `history`.
    pub fn push(&mut self, addr: u64, history: &mut RasHistory) {
        if self.stack.is_empty() {
            return;
        }
        self.tos = self.above(self.tos);
        self.stack[self.tos] = Some(addr);
        history.pushed = true;
    }

    /// Pops a return's predicted target, recording the pop in `history`.
    pub fn pop(&mut self, history: &mut RasHistory) -> Option<u64> {
        let entry = self.top();
        if self.stack.is_empty() {
            return entry;
        }
        history.popped = Some(PoppedEntry { tos: self.tos, entry });
        self.tos = self.below(self.tos);
        entry
    }

    /// The address a return would predict now.
    pub fn top(&self) -> Option<u64> {
        self.stack.get(self.tos).copied().flatten()
    }

    /// Undoes the operations a squashed prediction performed.
    pub fn squash(&mut self, history: RasHistory) {
        if history.pushed {
            self.tos = self.below(self.tos);
        }
        if let Some(popped) = history.popped {
            self.tos = popped.tos;
            self.stack[self.tos] = popped.entry;
        }
    }

    const fn above(&self, index: usize) -> usize {
        if index + 1 == self.stack.len() { 0 } else { index + 1 }
    }

    const fn below(&self, index: usize) -> usize {
        if index == 0 { self.stack.len() - 1 } else { index - 1 }
    }
}
