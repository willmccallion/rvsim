//! `misa` parsed from the single-letter part of an ISA string.

use crate::isa::csr;
use crate::isa::misa::Misa;

fn parse(isa: &str) -> Result<u64, String> {
    isa.parse::<Misa>().map(Misa::bits).map_err(|e| e.to_string())
}

#[test]
fn the_default_isa_string_gives_the_default_misa() {
    assert_eq!(parse("RV64IMAFDCB"), Ok(csr::MISA_DEFAULT_RV64GCB));
    assert_eq!(parse("RV64IMAFDC"), Ok(csr::MISA_DEFAULT_RV64GCB & !csr::MISA_EXT_B));
}

#[test]
fn g_stands_for_imafd_and_case_does_not_matter() {
    assert_eq!(parse("rv64gc"), parse("RV64IMAFDC"));
}

#[test]
fn a_string_without_c_leaves_the_c_bit_clear() {
    let misa = parse("RV64IMAFD").expect("valid ISA string");

    assert_eq!(misa & csr::MISA_EXT_C, 0);
    assert_eq!(misa, csr::MISA_DEFAULT_RV64GCB & !csr::MISA_EXT_C & !csr::MISA_EXT_B);
}

#[test]
fn the_vector_extension_sets_the_v_bit() {
    let misa = parse("RV64GCV").expect("valid ISA string");

    assert_eq!(misa & csr::MISA_EXT_V, csr::MISA_EXT_V);
}

#[test]
fn strings_the_hart_cannot_implement_are_rejected() {
    for isa in ["RV32IMAFDC", "RV64", "RV64MAFD", "RV64IQ", "RV64ID", "0x800000000014112d"] {
        assert!(parse(isa).is_err(), "{isa} should be rejected");
    }
}

#[test]
fn misa_formats_back_to_a_canonical_lowercase_isa_string() {
    let misa: Misa = "RV64GCBV".parse().expect("valid ISA string");

    assert_eq!(misa.isa_string(), "rv64imafdcbv");
}

#[test]
fn b_is_set_only_when_the_string_names_it() {
    for (isa, has_b) in [("RV64I", false), ("rv64gcv", false), ("RV64IB", true), ("RV64GCB", true)]
    {
        let misa = parse(isa).expect("valid ISA string");

        assert_eq!(misa & csr::MISA_EXT_B != 0, has_b, "{isa}");
    }
}
