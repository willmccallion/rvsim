//! The target description GDB asks for: which registers the hart has and
//! how they are numbered, in GDB's `riscv:rv64` layout.

use crate::isa::csr;

/// GDB's number for `pc`, after `x0` to `x31`.
pub const PC_REGNUM: u32 = 32;
/// GDB's number for `f0`; `f1` to `f31` follow.
pub const FIRST_FP_REGNUM: u32 = 33;
/// GDB's number for the CSR at address 0; a CSR's number is this plus
/// its address.
pub const FIRST_CSR_REGNUM: u32 = 65;

/// The CSRs the description names, by address, in the order they appear.
pub const CSRS: &[(&str, u32)] = &[
    ("fflags", csr::FFLAGS.as_u32()),
    ("frm", csr::FRM.as_u32()),
    ("fcsr", csr::FCSR.as_u32()),
    ("sstatus", 0x100),
    ("sie", 0x104),
    ("stvec", 0x105),
    ("sscratch", 0x140),
    ("sepc", 0x141),
    ("scause", 0x142),
    ("stval", 0x143),
    ("sip", 0x144),
    ("satp", 0x180),
    ("mstatus", 0x300),
    ("misa", 0x301),
    ("medeleg", 0x302),
    ("mideleg", 0x303),
    ("mie", 0x304),
    ("mtvec", 0x305),
    ("mscratch", 0x340),
    ("mepc", 0x341),
    ("mcause", 0x342),
    ("mtval", 0x343),
    ("mip", 0x344),
    ("cycle", 0xC00),
    ("time", 0xC01),
    ("instret", 0xC02),
    ("mhartid", 0xF14),
];

const X_NAMES: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "fp", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

/// The `target.xml` GDB reads: the integer, floating-point and control
/// registers of a 64-bit hart.
#[must_use]
pub fn description() -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE target SYSTEM \"gdb-target.dtd\">\n<target version=\"1.0\">\n  <architecture>riscv:rv64</architecture>\n  <feature name=\"org.gnu.gdb.riscv.cpu\">\n",
    );
    for (n, name) in X_NAMES.iter().enumerate() {
        let kind = match n {
            1 => "code_ptr",
            2 | 3 | 4 | 8 => "data_ptr",
            _ => "int",
        };
        xml.push_str(&reg(name, 64, kind, n as u32));
    }
    xml.push_str(&reg("pc", 64, "code_ptr", PC_REGNUM));
    xml.push_str("  </feature>\n  <feature name=\"org.gnu.gdb.riscv.fpu\">\n");
    for n in 0..32 {
        xml.push_str(&reg(&format!("f{n}"), 64, "ieee_double", FIRST_FP_REGNUM + n));
    }
    for (name, addr) in CSRS.iter().take(3) {
        xml.push_str(&reg(name, 32, "int", FIRST_CSR_REGNUM + addr));
    }
    xml.push_str("  </feature>\n  <feature name=\"org.gnu.gdb.riscv.csr\">\n");
    for (name, addr) in CSRS.iter().skip(3) {
        xml.push_str(&reg(name, 64, "int", FIRST_CSR_REGNUM + addr));
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
}
