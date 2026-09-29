//! Unit tests for the Zicboz / Zicbom CSR gate helpers.

use rvsim_core::core::arch::csr::{
    CboInvalAction, MENVCFG_CBCFE, MENVCFG_CBIE_SHIFT, MENVCFG_CBZE, SENVCFG_CBCFE, SENVCFG_CBZE,
    cbo_inval_action, cbocf_allowed, cboz_allowed,
};
use rvsim_core::isa::privileged::PrivilegeMode;

const CBIE_ILLEGAL: u64 = 0b00 << MENVCFG_CBIE_SHIFT;
const CBIE_FLUSH: u64 = 0b01 << MENVCFG_CBIE_SHIFT;
const CBIE_INVAL: u64 = 0b11 << MENVCFG_CBIE_SHIFT;

#[test]
fn cboz_machine_mode_always_allowed() {
    assert!(cboz_allowed(0, 0, PrivilegeMode::Machine));
    assert!(cboz_allowed(MENVCFG_CBZE, 0, PrivilegeMode::Machine));
}

#[test]
fn cboz_supervisor_requires_menvcfg_cbze() {
    assert!(!cboz_allowed(0, 0, PrivilegeMode::Supervisor));
    assert!(cboz_allowed(MENVCFG_CBZE, 0, PrivilegeMode::Supervisor));
    // senvcfg.CBZE alone doesn't grant S-mode access.
    assert!(!cboz_allowed(0, SENVCFG_CBZE, PrivilegeMode::Supervisor));
}

#[test]
fn cboz_user_requires_both_bits() {
    assert!(!cboz_allowed(0, 0, PrivilegeMode::User));
    assert!(!cboz_allowed(MENVCFG_CBZE, 0, PrivilegeMode::User));
    assert!(!cboz_allowed(0, SENVCFG_CBZE, PrivilegeMode::User));
    assert!(cboz_allowed(MENVCFG_CBZE, SENVCFG_CBZE, PrivilegeMode::User));
}

#[test]
fn cbocf_machine_mode_always_allowed() {
    assert!(cbocf_allowed(0, 0, PrivilegeMode::Machine));
}

#[test]
fn cbocf_supervisor_requires_menvcfg_cbcfe() {
    assert!(!cbocf_allowed(0, 0, PrivilegeMode::Supervisor));
    assert!(cbocf_allowed(MENVCFG_CBCFE, 0, PrivilegeMode::Supervisor));
}

#[test]
fn cbocf_user_requires_both_cbcfe_bits() {
    assert!(!cbocf_allowed(MENVCFG_CBCFE, 0, PrivilegeMode::User));
    assert!(!cbocf_allowed(0, SENVCFG_CBCFE, PrivilegeMode::User));
    assert!(cbocf_allowed(MENVCFG_CBCFE, SENVCFG_CBCFE, PrivilegeMode::User));
}

#[test]
fn cbo_inval_machine_mode_always_invalidates() {
    assert_eq!(cbo_inval_action(0, 0, PrivilegeMode::Machine), CboInvalAction::Invalidate);
}

#[test]
fn cbo_inval_supervisor_reads_menvcfg_only() {
    assert_eq!(
        cbo_inval_action(CBIE_ILLEGAL, 0, PrivilegeMode::Supervisor),
        CboInvalAction::Illegal
    );
    assert_eq!(cbo_inval_action(CBIE_FLUSH, 0, PrivilegeMode::Supervisor), CboInvalAction::Flush);
    assert_eq!(
        cbo_inval_action(CBIE_INVAL, 0, PrivilegeMode::Supervisor),
        CboInvalAction::Invalidate
    );
}

#[test]
fn cbo_inval_user_takes_most_restrictive() {
    // Both invalidate → invalidate.
    assert_eq!(
        cbo_inval_action(CBIE_INVAL, CBIE_INVAL, PrivilegeMode::User),
        CboInvalAction::Invalidate
    );
    // Flush in either → flush.
    assert_eq!(
        cbo_inval_action(CBIE_INVAL, CBIE_FLUSH, PrivilegeMode::User),
        CboInvalAction::Flush
    );
    assert_eq!(
        cbo_inval_action(CBIE_FLUSH, CBIE_INVAL, PrivilegeMode::User),
        CboInvalAction::Flush
    );
    // Illegal in either → illegal.
    assert_eq!(
        cbo_inval_action(CBIE_ILLEGAL, CBIE_INVAL, PrivilegeMode::User),
        CboInvalAction::Illegal
    );
    assert_eq!(
        cbo_inval_action(CBIE_INVAL, CBIE_ILLEGAL, PrivilegeMode::User),
        CboInvalAction::Illegal
    );
}

#[test]
fn cbo_inval_reserved_encoding_is_illegal() {
    let cbie_reserved = 0b10 << MENVCFG_CBIE_SHIFT;
    assert_eq!(
        cbo_inval_action(cbie_reserved, 0, PrivilegeMode::Supervisor),
        CboInvalAction::Illegal
    );
    assert_eq!(
        cbo_inval_action(cbie_reserved, cbie_reserved, PrivilegeMode::User),
        CboInvalAction::Illegal
    );
}
