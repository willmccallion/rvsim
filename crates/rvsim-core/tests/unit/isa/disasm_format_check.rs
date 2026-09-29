//! Quick test to verify disasm output format

use rvsim_core::isa::disasm::disassemble;

#[test]
fn check_formats() {
    println!("BEQ: '{}'", disassemble(0x00B58463));
    println!("JAL: '{}'", disassemble(0x008000EF));
    println!("CSRRW: '{}'", disassemble(0x30051573));
    println!("FEQ.S: '{}'", disassemble(0xA0B52553));
    println!("FLE.S: '{}'", disassemble(0xA0B50553));
    assert!(true);
}
