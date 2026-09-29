//! Per-set recency order, shared by the LRU and MRU policies.

/// Each set's ways ordered from most to least recently used.
#[derive(Debug)]
pub(super) struct RecencyStacks {
    stacks: Vec<Vec<usize>>,
}

impl RecencyStacks {
    /// `sets` stacks of `ways` ways, initially in way order.
    pub(super) fn new(sets: usize, ways: usize) -> Self {
        Self { stacks: (0..sets).map(|_| (0..ways).collect()).collect() }
    }

    /// Makes `way` the most recently used way of `set`.
    pub(super) fn touch(&mut self, set: usize, way: usize) {
        let stack = &mut self.stacks[set];
        if let Some(pos) = stack.iter().position(|&x| x == way) {
            let _ = stack.remove(pos);
        }
        stack.insert(0, way);
    }

    /// The most recently used way of `set`.
    pub(super) fn most_recent(&self, set: usize) -> usize {
        self.stacks[set].first().copied().unwrap_or(0)
    }

    /// The least recently used way of `set`.
    pub(super) fn least_recent(&self, set: usize) -> usize {
        self.stacks[set].last().copied().unwrap_or(0)
    }
}
