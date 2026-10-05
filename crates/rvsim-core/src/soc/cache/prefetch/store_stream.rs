//! Store-miss stream prefetcher.
//!
//! The Cortex-A72 manages "prefetching on store accesses" with "a PA based
//! prefetcher [that] only prefetches to the L2 cache", fed by the L1D's
//! `ReadUnique` transactions (TRM §6.4.9, `CPUACTLR_EL1[42]`). This one sees
//! the store misses that start a fetch for write permission, finds runs of
//! them to adjacent lines inside one 4 KiB physical page, and keeps each
//! run `l2_lines` lines ahead. Without a translation it cannot know a
//! larger page, so it stays inside the smallest.

use crate::common::PAGE_SHIFT;

/// A run of store misses in one page.
#[derive(Clone, Copy, Debug)]
struct StoreRun {
    page: u64,
    last_line: u64,
    /// +1 or -1 once two adjacent misses have set it, 0 before.
    direction: i64,
    /// The furthest line requested.
    ahead: Option<u64>,
}

/// The L1D's store-miss prefetcher.
#[derive(Debug)]
pub struct StoreStreamPrefetcher {
    /// Runs, least recently extended first.
    runs: Vec<StoreRun>,
    capacity: usize,
    l2_lines: usize,
    line_bytes: u64,
}

impl StoreStreamPrefetcher {
    /// A prefetcher tracking `streams` runs and keeping each `l2_lines`
    /// lines ahead.
    #[must_use]
    pub fn new(line_bytes: usize, streams: usize, l2_lines: usize) -> Self {
        Self {
            runs: Vec::with_capacity(streams.max(1)),
            capacity: streams.max(1),
            l2_lines,
            line_bytes: line_bytes as u64,
        }
    }

    /// Observes a store miss to `addr` and returns the lines to prefetch
    /// into the L2, nearest first. A miss next to a run's last one extends
    /// it; two in the same direction confirm it.
    pub fn observe(&mut self, addr: u64) -> Vec<u64> {
        let line = addr & !(self.line_bytes - 1);
        let page = addr >> PAGE_SHIFT;
        let adjacent = self
            .runs
            .iter()
            .position(|run| run.page == page && line.abs_diff(run.last_line) == self.line_bytes);
        let Some(index) = adjacent else {
            self.start_run(page, line);
            return Vec::new();
        };
        let mut run = self.runs.remove(index);
        let direction = if line > run.last_line { 1 } else { -1 };
        let confirmed = run.direction == direction;
        if !confirmed {
            run.direction = direction;
            run.ahead = None;
        }
        run.last_line = line;
        let prefetches = if confirmed { self.extend(&mut run) } else { Vec::new() };
        self.runs.push(run);
        prefetches
    }

    fn start_run(&mut self, page: u64, line: u64) {
        if self.runs.len() == self.capacity {
            let _ = self.runs.remove(0);
        }
        self.runs.push(StoreRun { page, last_line: line, direction: 0, ahead: None });
    }

    /// The lines of `run`'s page up to `l2_lines` past its last miss that
    /// it has not requested yet.
    fn extend(&self, run: &mut StoreRun) -> Vec<u64> {
        let step = self.line_bytes as i64 * run.direction;
        let mut prefetches = Vec::new();
        for k in 1..=self.l2_lines as i64 {
            let target = (run.last_line as i64).wrapping_add(step * k) as u64;
            if target >> PAGE_SHIFT != run.page {
                break;
            }
            let requested = run.ahead.is_some_and(|ahead| {
                (target as i64).wrapping_sub(ahead as i64).signum() != run.direction
            });
            if !requested {
                prefetches.push(target);
                run.ahead = Some(target);
            }
        }
        prefetches
    }
}
