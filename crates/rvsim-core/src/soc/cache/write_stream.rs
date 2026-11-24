//! Write streaming: a run of whole-line writes to consecutive lines, in
//! either direction, stops allocating once it is long enough, as the
//! Cortex-A72's load/store unit decides for a copy or a fill
//! (`CPUACTLR_EL1[28:25]`). Writes that cover part of a line break the run;
//! reads do not, so a copy's loads leave its stores' run intact.

use crate::common::LineAddr;
use crate::config::WriteStreamingConfig;

/// Where a whole-line write that misses the L1D is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamPlacement {
    /// Fetched into the L1D as any write miss.
    L1,
    /// Not fetched; kept by the L2.
    L2,
    /// Not fetched; kept by no cache.
    Memory,
}

/// Which way a run moves through memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

/// Counts the current run of whole-line writes to consecutive lines.
#[derive(Clone, Debug)]
pub struct WriteStreamDetector {
    config: WriteStreamingConfig,
    line_bytes: u64,
    last: Option<LineAddr>,
    direction: Option<Direction>,
    /// Lines in the run, the last included.
    run: usize,
}

impl WriteStreamDetector {
    /// A detector over lines of `line_bytes`.
    #[must_use]
    pub const fn new(config: WriteStreamingConfig, line_bytes: u64) -> Self {
        Self { config, line_bytes, last: None, direction: None, run: 0 }
    }

    /// Counts a whole-line write to `line` and says where it goes if it
    /// misses.
    pub fn whole_line(&mut self, line: LineAddr) -> StreamPlacement {
        let step = self.last.and_then(|last| self.direction_from(last, line));
        match step {
            Some(direction) if self.direction.is_none_or(|run| run == direction) => {
                self.run += 1;
                self.direction = Some(direction);
            }
            Some(direction) => {
                self.run = 2;
                self.direction = Some(direction);
            }
            None => {
                self.run = 1;
                self.direction = None;
            }
        }
        self.last = Some(line);
        self.placement()
    }

    /// A write to part of a line ends the run.
    pub const fn partial_line(&mut self) {
        self.last = None;
        self.direction = None;
        self.run = 0;
    }

    /// The direction from `last` to `line` when they are adjacent lines.
    fn direction_from(&self, last: LineAddr, line: LineAddr) -> Option<Direction> {
        if line.val().checked_sub(last.val()) == Some(self.line_bytes) {
            Some(Direction::Up)
        } else if last.val().checked_sub(line.val()) == Some(self.line_bytes) {
            Some(Direction::Down)
        } else {
            None
        }
    }

    const fn placement(&self) -> StreamPlacement {
        if self.run >= self.config.skip_l2_from.get() {
            StreamPlacement::Memory
        } else if self.run >= self.config.skip_l1_from.get() {
            StreamPlacement::L2
        } else {
            StreamPlacement::L1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::PhysAddr;
    use std::num::NonZeroUsize;

    const LINE_BYTES: u64 = 64;

    /// The Cortex-A72's reset thresholds, over 64-byte lines.
    fn detector() -> WriteStreamDetector {
        let config = WriteStreamingConfig {
            skip_l1_from: NonZeroUsize::new(4).unwrap(),
            skip_l2_from: NonZeroUsize::new(12).unwrap(),
        };
        WriteStreamDetector::new(config, LINE_BYTES)
    }

    /// Line number `index` as an address.
    fn line(index: u64) -> LineAddr {
        LineAddr::from_phys(PhysAddr::new(index * LINE_BYTES), LINE_BYTES)
    }

    fn placements(detector: &mut WriteStreamDetector, lines: &[u64]) -> Vec<StreamPlacement> {
        lines.iter().map(|&index| detector.whole_line(line(index))).collect()
    }

    #[test]
    fn a_rising_run_skips_the_l1_from_its_fourth_line_and_the_l2_from_its_twelfth() {
        let mut detector = detector();

        let placed = placements(&mut detector, &(100..113).collect::<Vec<_>>());

        let expected: Vec<_> = (1..=13)
            .map(|n| match n {
                ..4 => StreamPlacement::L1,
                4..12 => StreamPlacement::L2,
                _ => StreamPlacement::Memory,
            })
            .collect();
        assert_eq!(placed, expected);
    }

    #[test]
    fn a_falling_run_streams_too() {
        let mut detector = detector();

        let placed = placements(&mut detector, &[50, 49, 48, 47]);

        assert_eq!(placed.last(), Some(&StreamPlacement::L2));
    }

    #[test]
    fn a_write_to_a_distant_line_starts_a_new_run() {
        let mut detector = detector();
        let _ = placements(&mut detector, &[10, 11, 12, 13]);

        let placed = placements(&mut detector, &[500]);

        assert_eq!(placed, [StreamPlacement::L1]);
    }

    #[test]
    fn writing_the_same_line_again_starts_a_new_run() {
        let mut detector = detector();
        let _ = placements(&mut detector, &[10, 11, 12]);

        let placed = placements(&mut detector, &[12, 13]);

        assert_eq!(placed, [StreamPlacement::L1, StreamPlacement::L1]);
    }

    #[test]
    fn turning_round_starts_a_run_of_the_last_two_lines() {
        let mut detector = detector();
        let _ = placements(&mut detector, &[10, 11, 12, 13]);

        let placed = placements(&mut detector, &[12, 11, 10]);

        assert_eq!(placed, [StreamPlacement::L1, StreamPlacement::L1, StreamPlacement::L2]);
    }

    #[test]
    fn a_partial_line_write_ends_the_run() {
        let mut detector = detector();
        let _ = placements(&mut detector, &[10, 11, 12]);
        detector.partial_line();

        let placed = placements(&mut detector, &[13]);

        assert_eq!(placed, [StreamPlacement::L1]);
    }
}
