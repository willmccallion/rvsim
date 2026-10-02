//! `misa` parsed from an ISA string, as `pipeline.misa_override` takes it.

use crate::isa::csr;
use crate::isa::misa::Misa;

fn parse(isa: &str) -> Result<u64, String> {
    isa.parse::<Misa>().map(Misa::bits).map_err(|e| e.to_string())
}

#[test]
fn the_default_isa_string_gives_the_default_misa() {
    assert_eq!(parse("RV64IMAFDC"), Ok(csr::MISA_DEFAULT_RV64IMAFDC));
}

#[test]
fn g_stands_for_imafd_and_case_does_not_matter() {
    assert_eq!(parse("rv64gc"), parse("RV64IMAFDC"));
}

#[test]
fn a_string_without_c_leaves_the_c_bit_clear() {
    let misa = parse("RV64IMAFD").expect("valid ISA string");

    assert_eq!(misa & csr::MISA_EXT_C, 0);
    assert_eq!(misa, csr::MISA_DEFAULT_RV64IMAFDC & !csr::MISA_EXT_C);
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
    let misa: Misa = "RV64GCV".parse().expect("valid ISA string");

    assert_eq!(misa.isa_string(), "rv64imafdcv");
}
