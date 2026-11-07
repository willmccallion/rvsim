//! Conditional-branch comparisons.

use crate::exec::signals::BranchCond;

const MINUS_ONE: u64 = u64::MAX;

#[test]
fn equality_branches_compare_bits() {
    assert!(BranchCond::Eq.is_taken(5, 5));
    assert!(!BranchCond::Eq.is_taken(5, 6));
    assert!(BranchCond::Ne.is_taken(5, 6));
    assert!(!BranchCond::Ne.is_taken(5, 5));
}

#[test]
fn signed_branches_order_minus_one_below_one() {
    assert!(BranchCond::Lt.is_taken(MINUS_ONE, 1));
    assert!(!BranchCond::Ge.is_taken(MINUS_ONE, 1));
    assert!(BranchCond::Ge.is_taken(1, 1));
}

#[test]
fn unsigned_branches_order_minus_one_above_one() {
    assert!(!BranchCond::Ltu.is_taken(MINUS_ONE, 1));
    assert!(BranchCond::Geu.is_taken(MINUS_ONE, 1));
    assert!(BranchCond::Geu.is_taken(1, 1));
}
