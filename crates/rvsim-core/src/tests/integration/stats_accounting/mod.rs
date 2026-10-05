//! Every stat counts what its description says. Each check runs a small
//! program whose counts can be worked out by hand and asserts the stats
//! through a [`Recorder`], which notes every path it checked.

pub mod caches;
pub mod coherence;
pub mod commit;
pub mod fu;
pub mod pipeline;
pub mod predictors;
pub mod program;

use std::collections::BTreeSet;

use crate::Simulator;
use crate::sim::stats::Histogram;

/// Asserts stats and remembers which paths were checked.
#[derive(Default)]
pub struct Recorder {
    checked: BTreeSet<String>,
}

impl Recorder {
    /// Asserts that `path` reads `expected`.
    pub fn expect(&mut self, sim: &Simulator, path: &str, expected: u64, context: &str) {
        assert_eq!(read(sim, path), expected as f64, "{context}: {path}");
        self.note(path);
    }

    /// Asserts that the derived `path` reads `expected` to within rounding.
    pub fn expect_ratio(&mut self, sim: &Simulator, path: &str, expected: f64, context: &str) {
        let actual = read(sim, path);
        assert!((actual - expected).abs() < 1e-9, "{context}: {path} is {actual}, not {expected}");
        self.note(path);
    }

    /// The value of `path`, noting it as checked: for stats a check
    /// asserts through a relation with others rather than a fixed value.
    pub fn read(&mut self, sim: &Simulator, path: &str) -> u64 {
        self.note(path);
        read(sim, path) as u64
    }

    /// The histogram at `path`, noting it as checked.
    pub fn histogram(&mut self, sim: &Simulator, path: &str) -> Histogram {
        self.note(path);
        sim.stats()
            .histogram_at(path)
            .cloned()
            .unwrap_or_else(|| panic!("{path} has recorded nothing"))
    }

    fn note(&mut self, path: &str) {
        let _ = self.checked.insert(path.to_owned());
    }
}

fn read(sim: &Simulator, path: &str) -> f64 {
    sim.stats().get(path).unwrap_or_else(|| panic!("{path} is not a registered stat"))
}

/// Declares each check as a test of its own and lists them all in
/// `CHECKS`.
macro_rules! accounting_checks {
    ($($check:ident),* $(,)?) => {
        #[cfg(test)]
        mod each {
            $(
                #[test]
                fn $check() {
                    super::$check(&mut crate::tests::integration::stats_accounting::Recorder::default());
                }
            )*
        }

        /// Every check in this module.
        #[allow(dead_code)]
        pub const CHECKS: &[fn(&mut crate::tests::integration::stats_accounting::Recorder)] =
            &[$($check),*];
    };
}
pub(crate) use accounting_checks;
