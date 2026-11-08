//! The target description GDB asks for: which registers the hart has and
//! how they are numbered, in GDB's `riscv:rv64` layout.

use crate::isa::csr;
use crate::isa::reg::RegIdx;

/// GDB's number for `pc`, after `x0` to `x31`.
pub const PC_REGNUM: u32 = 32;
/// GDB's number for `f0`; `f1` to `f31` follow.
pub const FIRST_FP_REGNUM: u32 = 33;
/// GDB's number for the CSR at address 0; a CSR's number is this plus
/// its address.
pub const FIRST_CSR_REGNUM: u32 = 65;

/// GDB's name for integer register `idx`: its ABI name, except that
/// GDB's `riscv:rv64` feature names `x8` `fp`.
fn xreg_name(idx: RegIdx) -> &'static str {
    if idx == REG_FP { "fp" } else { idx.int_abi_name() }
}

/// The CSRs GDB's `riscv:rv64` layout puts in its FPU feature, at 32 bits.
const FP_CSRS: [csr::CsrAddr; 3] = [csr::FFLAGS, csr::FRM, csr::FCSR];

/// `x8`, the frame pointer.
const REG_FP: RegIdx = RegIdx::new(8);

/// The `target.xml` GDB reads: the integer, floating-point and control
/// registers of a 64-bit hart.
#[must_use]
pub fn description() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE target SYSTEM \"gdb-target.dtd\">\n<target version=\"1.0\">\n  <architecture>riscv:rv64</architecture>\n  <feature name=\"org.gnu.gdb.riscv.cpu\">\n",
    );
    for (n, idx) in (0u32..).zip(RegIdx::all()) {
        let kind = match n {
            1 => "code_ptr",
            2 | 3 | 4 | 8 => "data_ptr",
            _ => "int",
        };
        xml.push_str(&reg(xreg_name(idx), 64, kind, n));
    }
    xml.push_str(&reg("pc", 64, "code_ptr", PC_REGNUM));
    xml.push_str("  </feature>\n  <feature name=\"org.gnu.gdb.riscv.fpu\">\n");
    for n in 0..32 {
        xml.push_str(&reg(&format!("f{n}"), 64, "ieee_double", FIRST_FP_REGNUM + n));
    }
    let (fp_csrs, other_csrs): (Vec<_>, Vec<_>) =
        csr::NAMED.iter().partition(|(_, addr)| FP_CSRS.contains(addr));
    for (name, addr) in fp_csrs {
        xml.push_str(&reg(name, 32, "int", FIRST_CSR_REGNUM + addr.as_u32()));
    }
    xml.push_str("  </feature>\n  <feature name=\"org.gnu.gdb.riscv.csr\">\n");
    for (name, addr) in other_csrs {
        xml.push_str(&reg(name, 64, "int", FIRST_CSR_REGNUM + addr.as_u32()));
    }
    xml.push_str("  </feature>\n</target>\n");
    xml
}

/// One register's line of the description.
fn reg(name: &str, bitsize: u32, kind: &str, regnum: u32) -> String {
    format!(
        "    <reg name=\"{name}\" bitsize=\"{bitsize}\" type=\"{kind}\" regnum=\"{regnum}\"/>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_description_numbers_registers_as_gdb_expects() {
        let xml = description();

        assert!(xml.contains("name=\"pc\" bitsize=\"64\" type=\"code_ptr\" regnum=\"32\""));
        assert!(xml.contains("name=\"f0\" bitsize=\"64\" type=\"ieee_double\" regnum=\"33\""));
        assert!(xml.contains(&format!(
            "name=\"mstatus\" bitsize=\"64\" type=\"int\" regnum=\"{}\"",
            65 + 0x300
        )));
        assert!(xml.contains("<architecture>riscv:rv64</architecture>"));
    }

    #[test]
    fn every_named_csr_is_described_with_the_fp_ones_at_32_bits() {
        let xml = description();

        assert!(xml.contains(&format!(
            "name=\"fflags\" bitsize=\"32\" type=\"int\" regnum=\"{}\"",
            65 + 0x001
        )));
        for (name, _) in csr::NAMED {
            assert!(xml.contains(&format!("name=\"{name}\"")), "{name} is described");
        }
    }

    #[test]
    fn integer_registers_carry_their_abi_names_with_x8_as_fp() {
        let xml = description();

        assert!(xml.contains("name=\"zero\" bitsize=\"64\" type=\"int\" regnum=\"0\""));
        assert!(xml.contains("name=\"fp\" bitsize=\"64\" type=\"data_ptr\" regnum=\"8\""));
        assert!(xml.contains("name=\"a0\" bitsize=\"64\" type=\"int\" regnum=\"10\""));
        assert!(xml.contains("name=\"t6\" bitsize=\"64\" type=\"int\" regnum=\"31\""));
    }
}
