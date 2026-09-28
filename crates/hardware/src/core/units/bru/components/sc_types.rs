//! What TAGE tells the statistical corrector about its prediction.

/// How confident TAGE's longest match is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TageConfLevel {
    /// A saturated counter.
    High,
    /// A tagged counter one step from saturation, `|2*ctr+1| == 5`.
    Medium,
    /// A weak tagged counter, or an unsaturated bimodal one.
    Low,
    /// A tagged counter between weak and medium, `|2*ctr+1| == 3`.
    None,
}

impl TageConfLevel {
    /// The confidence of a 3-bit tagged provider counter.
    pub const fn from_tagged_ctr(ctr: i8) -> Self {
        let centred = (2 * ctr as i32 + 1).unsigned_abs();
        match centred {
            c if c >= 7 => Self::High,
            5 => Self::Medium,
            1 => Self::Low,
            _ => Self::None,
        }
    }

    /// The confidence of the bimodal when it provides the prediction.
    pub const fn from_bimodal(saturated: bool) -> Self {
        if saturated { Self::High } else { Self::Low }
    }
}

/// TAGE's prediction as the statistical corrector indexes and decides by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TageScMeta {
    /// Confidence of the longest match (or the bimodal without one).
    pub conf: TageConfLevel,
    /// Provider bank (0 = bimodal base, 1+ = tagged bank).
    pub provider_bank: usize,
    /// Whether an alternate tagged bank matched.
    pub alt_bank_present: bool,
    /// TAGE's prediction, after `USE_ALT_ON_NA`.
    pub pred_taken: bool,
    /// The longest match and the alternate predict differently.
    pub provider_disagrees_with_alt: bool,
}
