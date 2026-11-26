//! Decode reads and writes exactly the scalar registers that riscv-opcodes,
//! the machine-readable list of RISC-V encodings, gives each instruction
//! of the extensions rvsim implements. See `tests/data/riscv-opcodes`.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::exec::decode::{DecodeFault, DecodedInst, decode_inst};
use crate::isa::reg::{ArchReg, RegFile, RegIdx};

const INSTR_DICT: &str = include_str!("../../data/riscv-opcodes/instr_dict.json");
const ARG_LUT: &str = include_str!("../../data/riscv-opcodes/arg_lut.csv");

/// The riscv-opcodes extension files whose instructions rvsim implements
/// on RV64.
const IMPLEMENTED_EXTENSIONS: &[&str] = &[
    "rv_i",
    "rv64_i",
    "rv_m",
    "rv64_m",
    "rv_a",
    "rv64_a",
    "rv_f",
    "rv64_f",
    "rv_d",
    "rv64_d",
    "rv_zfh",
    "rv64_zfh",
    "rv_zfhmin",
    "rv_d_zfhmin",
    "rv_zicsr",
    "rv_zifencei",
    "rv_zicbo",
    "rv_zba",
    "rv64_zba",
    "rv_zbb",
    "rv64_zbb",
    "rv_zbc",
    "rv_zbs",
    "rv64_zbs",
    "rv_zbkb",
    "rv64_zbkb",
    "rv_zbkc",
    "rv_zbkx",
    "rv_v",
    "rv_zvbb",
    "rv_zvbc",
    "rv_zvkg",
    "rv_zvkned",
    "rv_zvknha",
    "rv_zvknhb",
    "rv_zvksed",
    "rv_zvksh",
    "rv_s",
    "rv_system",
];

/// Instructions that decode to a trap by design.
const TRAPS_BY_DESIGN: &[&str] = &["ebreak"];

/// FENCE and FENCE.I list `rs1` and `rd`, which the spec reserves for
/// finer-grain fences and requires implementations to ignore.
const RESERVED_REGISTER_FIELDS: &[&str] = &["fence", "fence_i"];

/// Vector instructions whose scalar operand riscv-opcodes names `rs1` but
/// which read it from the FP registers, as every OPFVF scalar is.
const READS_FP_RS1: &[&str] = &["vfmerge_vfm", "vfmv_s_f", "vfmv_v_f", "vfwadd_wf", "vfwsub_wf"];

/// `vfmv.f.s`, which riscv-opcodes gives an `rd` but writes an FP register.
const WRITES_FP_RD: &[&str] = &["vfmv_f_s"];

/// The register each register field names: distinct and non-zero, and for
/// vector registers aligned for the eight-register groups of whole-register
/// moves and accesses.
const RD: u32 = 10;
const RS1: u32 = 11;
const RS2: u32 = 12;
const RS3: u32 = 13;
const VD: u32 = 8;
const VS2: u32 = 16;
const VS1: u32 = 24;

/// SEW=64 and LMUL=1: every vector access's register group fits, since a
/// narrower element only shrinks it.
const VTYPE_E64_M1: u64 = 0b011 << 3;

#[derive(Deserialize)]
struct Opcode {
    variable_fields: Vec<String>,
    extension: Vec<String>,
    #[serde(rename = "match")]
    fixed_bits: String,
}

/// The scalar register files an instruction reads its sources from and
/// writes its result to.
#[derive(Debug, PartialEq, Eq)]
struct ScalarOperands {
    rs1: Option<RegFile>,
    rs2: Option<RegFile>,
    rs3: Option<RegFile>,
    rd: Option<RegFile>,
}

fn implemented_opcodes() -> BTreeMap<String, Opcode> {
    let opcodes: BTreeMap<String, Opcode> =
        serde_json::from_str(INSTR_DICT).expect("instr_dict.json parses");
    opcodes
        .into_iter()
        .filter(|(_, opcode)| {
            opcode.extension.iter().any(|ext| IMPLEMENTED_EXTENSIONS.contains(&ext.as_str()))
        })
        .collect()
}

/// Each field's `(msb, lsb)` in the instruction word.
fn field_positions() -> HashMap<&'static str, (u32, u32)> {
    ARG_LUT
        .lines()
        .map(|line| {
            let mut columns = line.split(',').map(str::trim);
            let name = columns.next().expect("a name").trim_matches('"');
            let mut bit = || columns.next().expect("a bit").parse().expect("a bit number");
            (name, (bit(), bit()))
        })
        .collect()
}

/// The value placed in field `name`: the register it names, unmasked for
/// `vm`, and 0 for an immediate, CSR, rounding mode or ordering.
fn field_value(name: &str) -> u32 {
    match name {
        "rd" | "fd" => RD,
        "rs1" | "fs1" => RS1,
        "rs2" | "fs2" => RS2,
        "rs3" | "fs3" => RS3,
        "vd" | "vs3" => VD,
        "vs1" => VS1,
        "vs2" => VS2,
        "vm" => 1,
        _ => 0,
    }
}

fn encode(opcode: &Opcode, positions: &HashMap<&str, (u32, u32)>) -> u32 {
    let fixed = opcode.fixed_bits.trim_start_matches("0x");
    let fixed = u32::from_str_radix(fixed, 16).expect("a hex match value");
    opcode.variable_fields.iter().fold(fixed, |word, name| {
        let (msb, lsb) = positions[name.as_str()];
        let width_mask = (1 << (msb - lsb + 1)) - 1;
        word | (field_value(name) & width_mask) << lsb
    })
}

fn decode(opcode: &Opcode, positions: &HashMap<&str, (u32, u32)>) -> DecodedInst {
    decode_inst(encode(opcode, positions), VTYPE_E64_M1)
}

/// The register file the field named `int` or `fp` puts an operand in.
fn listed_file(fields: &[String], int: &str, fp: &str) -> Option<RegFile> {
    if fields.iter().any(|field| field == int) {
        Some(RegFile::Int)
    } else if fields.iter().any(|field| field == fp) {
        Some(RegFile::Fp)
    } else {
        None
    }
}

/// The operands riscv-opcodes lists for `name`, with its known errors
/// corrected.
fn expected_operands(name: &str, opcode: &Opcode) -> ScalarOperands {
    if RESERVED_REGISTER_FIELDS.contains(&name) {
        return ScalarOperands { rs1: None, rs2: None, rs3: None, rd: None };
    }
    let fields = &opcode.variable_fields;
    let in_fp =
        |file: Option<RegFile>, is_fp: bool| file.map(|f| if is_fp { RegFile::Fp } else { f });
    ScalarOperands {
        rs1: in_fp(listed_file(fields, "rs1", "fs1"), READS_FP_RS1.contains(&name)),
        rs2: listed_file(fields, "rs2", "fs2"),
        rs3: listed_file(fields, "rs3", "fs3"),
        rd: in_fp(listed_file(fields, "rd", "fd"), WRITES_FP_RD.contains(&name)),
    }
}

const fn file_of(reg: ArchReg) -> RegFile {
    match reg {
        ArchReg::Int(_) => RegFile::Int,
        ArchReg::Fp(_) => RegFile::Fp,
    }
}

fn decoded_operands(decoded: &DecodedInst) -> ScalarOperands {
    let ctrl = &decoded.ctrl;
    ScalarOperands {
        rs1: ctrl.rs1_file,
        rs2: ctrl.rs2_file,
        rs3: ctrl.rs3_file,
        rd: ctrl.dest(RegIdx::new(RD as u8)).map(file_of),
    }
}

/// Fails listing every message `check` returns.
fn assert_all(check: impl Fn(&str, &Opcode, &DecodedInst) -> Option<String>) {
    let positions = field_positions();
    let failures: Vec<String> = implemented_opcodes()
        .iter()
        .filter_map(|(name, opcode)| check(name, opcode, &decode(opcode, &positions)))
        .collect();
    assert!(failures.is_empty(), "{} instructions:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn every_implemented_extension_file_lists_instructions() {
    let opcodes = implemented_opcodes();

    for ext in IMPLEMENTED_EXTENSIONS {
        let listed = opcodes.values().any(|opcode| opcode.extension.iter().any(|e| e == ext));
        assert!(listed, "riscv-opcodes lists no instruction in {ext}");
    }
}

#[test]
fn every_implemented_instruction_decodes() {
    assert_all(|name, _, decoded| match decoded.fault {
        Some(DecodeFault::Breakpoint) if TRAPS_BY_DESIGN.contains(&name) => None,
        Some(fault) => Some(format!("{name}: {fault:?}")),
        None => None,
    });
}

#[test]
fn every_instruction_uses_the_registers_riscv_opcodes_lists() {
    assert_all(|name, opcode, decoded| {
        let expected = expected_operands(name, opcode);
        let actual = decoded_operands(decoded);
        (decoded.fault.is_none() && actual != expected)
            .then(|| format!("{name}: expected {expected:?}, decoded {actual:?}"))
    });
}

#[test]
fn every_correction_matches_a_field_riscv_opcodes_still_lists() {
    let opcodes = implemented_opcodes();
    let lists = |name: &str, field: &str| {
        opcodes[name].variable_fields.iter().any(|listed| listed == field)
    };

    for name in RESERVED_REGISTER_FIELDS {
        assert!(lists(name, "rs1") && lists(name, "rd"), "{name}");
    }
    for name in READS_FP_RS1 {
        assert!(lists(name, "rs1"), "{name}");
    }
    for name in WRITES_FP_RD {
        assert!(lists(name, "rd"), "{name}");
    }
}
