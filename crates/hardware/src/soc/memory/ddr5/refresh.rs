//! Refresh cadence policies.
//!
//! DDR5 offers two refresh commands. `REFab` refreshes every bank of a rank
//! at once and holds the whole rank for tRFC1. `REFsb` refreshes one bank
//! in every bank group (a "bank set") for tRFCsb while the other banks keep
//! serving; the controller rotates through the sets so each bank is still
//! refreshed once per tREFI. A [`RefreshPolicy`] decides how often a rank
//! must refresh and which banks each command covers.

use std::fmt::Debug;

use crate::soc::memory::ddr5::timing::Ddr5Timing;

/// Bank layout of one rank, as the policy needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RankLayout {
    /// Bank groups per rank.
    pub bank_groups: u8,
    /// Banks per bank group.
    pub banks_per_group: u8,
}

impl RankLayout {
    /// Total banks in the rank.
    #[must_use]
    pub const fn bank_count(self) -> u32 {
        self.bank_groups as u32 * self.banks_per_group as u32
    }
}

/// The banks one refresh command covers and how long it holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshTarget {
    /// Bit `i` set means bank index `i` (`bank_group * banks_per_group + bank`)
    /// is refreshed.
    pub bank_mask: u64,
    /// Clocks the covered banks are unavailable after the command.
    pub duration: u64,
}

/// Decides refresh cadence and coverage for a rank.
pub trait RefreshPolicy: Debug + Send + Sync {
    /// Clocks between consecutive refresh commands on one rank.
    fn interval(&self, timing: &Ddr5Timing, layout: RankLayout) -> u64;

    /// Banks covered by the `sequence`-th refresh command on a rank
    /// (counting from zero) and the time they stay busy.
    fn target(&self, timing: &Ddr5Timing, layout: RankLayout, sequence: u64) -> RefreshTarget;
}

/// `REFab` every tREFI: the whole rank refreshes together.
#[derive(Debug, Default)]
pub struct AllBank;

impl RefreshPolicy for AllBank {
    fn interval(&self, timing: &Ddr5Timing, _layout: RankLayout) -> u64 {
        timing.t_refi
    }

    fn target(&self, timing: &Ddr5Timing, layout: RankLayout, _sequence: u64) -> RefreshTarget {
        RefreshTarget { bank_mask: bank_mask_all(layout), duration: timing.t_rfc1 }
    }
}

/// `REFsb` rotating through the bank sets.
///
/// One command every tREFI / banks-per-group, each covering the same bank
/// index in every bank group, so every bank is refreshed once per tREFI
/// while the rest of the rank stays available.
#[derive(Debug, Default)]
pub struct SameBank;

impl RefreshPolicy for SameBank {
    fn interval(&self, timing: &Ddr5Timing, layout: RankLayout) -> u64 {
        timing.t_refi / u64::from(layout.banks_per_group.max(1))
    }

    fn target(&self, timing: &Ddr5Timing, layout: RankLayout, sequence: u64) -> RefreshTarget {
        let set = sequence % u64::from(layout.banks_per_group.max(1));
        RefreshTarget { bank_mask: bank_mask_set(layout, set), duration: timing.t_rfcsb }
    }
}

/// Which [`RefreshPolicy`] a controller is built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Deserialize)]
pub enum RefreshKind {
    /// [`AllBank`].
    #[default]
    AllBank,
    /// [`SameBank`].
    SameBank,
}

impl RefreshKind {
    /// Instantiates the policy.
    #[must_use]
    pub fn build(self) -> Box<dyn RefreshPolicy> {
        match self {
            Self::AllBank => Box::new(AllBank),
            Self::SameBank => Box::new(SameBank),
        }
    }
}

/// Mask with every bank of the rank set.
#[must_use]
pub const fn bank_mask_all(layout: RankLayout) -> u64 {
    let count = layout.bank_count();
    if count >= 64 { u64::MAX } else { (1u64 << count) - 1 }
}

/// Mask with bank index `set` of every bank group set.
#[must_use]
pub const fn bank_mask_set(layout: RankLayout, set: u64) -> u64 {
    let mut mask = 0u64;
    let mut group = 0u64;
    while group < layout.bank_groups as u64 {
        let bank_index = group * layout.banks_per_group as u64 + set;
        if bank_index < 64 {
            mask |= 1u64 << bank_index;
        }
        group += 1;
    }
    mask
}
