//! The test support itself: the builders, the harness and the memory mock.

use crate::common::{PhysAddr, VirtAddr};
use crate::config::Config;
use crate::disasm::disassemble;
use crate::exec::signals::ControlSignals;
use crate::isa::encoding::rv64i::opcodes::OP_REG;
use crate::isa::reg::RegIdx;
use crate::soc::uncore::KERNEL_PANIC_GRACE_CYCLES;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::InstructionBuilder;
use crate::tests::support::builder::pipeline_state::{IdExBuilder, IfIdBuilder};
use crate::tests::support::harness::TestContext;
use crate::tests::support::mocks::memory::MockMemory;
use crate::tests::support::regs::{X0, X1, X2, X3, X5, X6};

#[test]
fn each_builder_method_assembles_the_instruction_it_names() {
    let i = InstructionBuilder::new;
    let cases = [
        (i().add(X1, X2, X3).build(), "add ra, sp, gp"),
        (i().sub(X1, X2, X3).build(), "sub ra, sp, gp"),
        (i().mul(X1, X2, X3).build(), "mul ra, sp, gp"),
        (i().div(X1, X2, X3).build(), "div ra, sp, gp"),
        (i().and(X1, X2, X3).build(), "and ra, sp, gp"),
        (i().or(X1, X2, X3).build(), "or ra, sp, gp"),
        (i().xor(X1, X2, X3).build(), "xor ra, sp, gp"),
        (i().sll(X1, X2, X3).build(), "sll ra, sp, gp"),
        (i().srl(X1, X2, X3).build(), "srl ra, sp, gp"),
        (i().sra(X1, X2, X3).build(), "sra ra, sp, gp"),
        (i().slt(X1, X2, X3).build(), "slt ra, sp, gp"),
        (i().sltu(X1, X2, X3).build(), "sltu ra, sp, gp"),
        (i().addw(X1, X2, X3).build(), "addw ra, sp, gp"),
        (i().subw(X1, X2, X3).build(), "subw ra, sp, gp"),
        (i().addi(X1, X2, -1).build(), "addi ra, sp, -1"),
        (i().addiw(X1, X2, 5).build(), "addiw ra, sp, 5"),
        (i().andi(X5, X6, 0xFF).build(), "andi t0, t1, 255"),
        (i().ori(X5, X6, 0xFF).build(), "ori t0, t1, 255"),
        (i().xori(X5, X6, 0xFF).build(), "xori t0, t1, 255"),
        (i().slti(X5, X6, 10).build(), "slti t0, t1, 10"),
        (i().sltiu(X5, X6, 10).build(), "sltiu t0, t1, 10"),
        (i().lbu(X1, X2, 3).build(), "lbu ra, 3(sp)"),
        (i().lw(X1, X2, 8).build(), "lw ra, 8(sp)"),
        (i().ld(X1, X2, 16).build(), "ld ra, 16(sp)"),
        (i().sb(X2, X3, 1).build(), "sb gp, 1(sp)"),
        (i().sw(X2, X3, 32).build(), "sw gp, 32(sp)"),
        (i().sd(X2, X3, -8).build(), "sd gp, -8(sp)"),
        (i().beq(X1, X2, 8).build(), "beq ra, sp, 8"),
        (i().bne(X1, X2, -8).build(), "bne ra, sp, -8"),
        (i().blt(X1, X2, 16).build(), "blt ra, sp, 16"),
        (i().bge(X1, X2, 8).build(), "bge ra, sp, 8"),
        (i().bltu(X1, X2, 8).build(), "bltu ra, sp, 8"),
        (i().bgeu(X1, X2, 8).build(), "bgeu ra, sp, 8"),
        (i().jal(X1, 2048).build(), "jal ra, 2048"),
        (i().jalr(X1, X2, 4).build(), "jalr ra, 4(sp)"),
        (i().lui(X1, 0x12345).build(), "lui ra, 0x12345"),
        (i().auipc(X1, 0x1).build(), "auipc ra, 0x1"),
        (i().csrrw(X1, 0x300, X2).build(), "csrrw ra, 0x300, sp"),
        (i().csrrs(X1, 0x341, X0).build(), "csrrs ra, 0x341, zero"),
        (i().csrrc(X0, 0x304, X5).build(), "csrrc zero, 0x304, t0"),
        (i().amoadd_d(X1, X2, X3).build(), "amoadd.d ra, gp, (sp)"),
        (i().lr_d(X1, X2).build(), "lr.d ra, (sp)"),
        (i().sc_d(X1, X2, X3).build(), "sc.d ra, gp, (sp)"),
        (i().nop().build(), "addi zero, zero, 0"),
        (i().opcode(OP_REG).rd(X1).rs1(X2).rs2(X3).funct7(0b010_0000).build(), "sub ra, sp, gp"),
    ];

    for (word, asm) in cases {
        assert_eq!(disassemble(word), asm, "{word:#010x}");
    }
}

#[test]
fn ifid_builder_defaults_and_setters() {
    let entry = IfIdBuilder::new().pc(0x1000).inst(0xDEADBEEF).predicted(0x2000).build();
    assert_eq!(entry.pc, VirtAddr::new(0x1000));
    assert_eq!(entry.bits, 0xDEADBEEF);
    assert_eq!(entry.predicted_target, Some(VirtAddr::new(0x2000)));
}

#[test]
fn ifid_builder_defaults_are_zero() {
    let entry = IfIdBuilder::new().build();
    assert_eq!(entry.pc, VirtAddr::new(0));
    assert_eq!(entry.bits, 0);
    assert_eq!(entry.predicted_target, None);
}

#[test]
fn idex_builder_full_chain() {
    let ctrl = ControlSignals { reg_write: true, ..Default::default() };
    let entry = IdExBuilder::new()
        .pc(0x2000)
        .inst(0x12345678)
        .rs1(X1, 100)
        .rs2(X2, 200)
        .rd(X3)
        .imm(42)
        .control(ctrl)
        .build();
    assert_eq!(entry.inst.pc, VirtAddr::new(0x2000));
    assert_eq!(entry.inst.bits, 0x12345678);
    assert_eq!(entry.inst.rs1, RegIdx::new(1));
    assert_eq!(entry.inst.rv1, 100);
    assert_eq!(entry.inst.rs2, RegIdx::new(2));
    assert_eq!(entry.inst.rv2, 200);
    assert_eq!(entry.inst.rd, RegIdx::new(3));
    assert_eq!(entry.inst.imm, 42);
    assert!(entry.inst.ctrl.reg_write);
}

#[test]
fn harness_boot_default_pc() {
    let ctx = TestContext::new();
    assert_eq!(ctx.state().harts[0].pc, VirtAddr::new(0x8000_0000), "CPU should start at RAM base");
}

#[test]
fn harness_with_memory_adds_device() {
    let mut ctx = TestContext::new().with_memory(4096, 0x1000);
    ctx.sim.probe_mem_store(PhysAddr::new(0x1000), 0xDEAD_BEEF, ProbeWidth::B4);
    assert_eq!(ctx.sim.probe_mem_load(PhysAddr::new(0x1000), ProbeWidth::B4), 0xDEAD_BEEF);
}

#[test]
fn harness_load_program_writes_instructions_and_sets_pc() {
    let nop = InstructionBuilder::new().nop().build();
    let addi = InstructionBuilder::new().addi(X1, X0, 42).build();
    let program = [nop, addi];

    let mut ctx = TestContext::new().with_memory(4096, 0x1000).load_program(0x1000, &program);

    assert_eq!(ctx.state().harts[0].pc, VirtAddr::new(0x1000), "PC should be set to program base");
    assert_eq!(ctx.sim.probe_mem_load(PhysAddr::new(0x1000), ProbeWidth::B4) as u32, nop);
    assert_eq!(ctx.sim.probe_mem_load(PhysAddr::new(0x1004), ProbeWidth::B4) as u32, addi);
}

#[test]
fn harness_set_and_get_reg() {
    let mut ctx = TestContext::new();
    ctx.set_reg(X5, 0xDEAD);
    assert_eq!(ctx.get_reg(X5), 0xDEAD);
}

/// A context spinning on `j .` whose UART has printed "Kernel panic", so a
/// tick fails once the panic grace period ends.
fn context_heading_for_a_kernel_panic() -> TestContext {
    let spin = InstructionBuilder::new().jal(X0, 0).build();
    let mut ctx = TestContext::new().with_memory(4096, 0x1000).load_program(0x1000, &[spin]);
    let uart = PhysAddr::new(Config::minimal().system.uart_base.val());
    for byte in b"Kernel panic" {
        ctx.sim.probe_mem_store(uart, u64::from(*byte), ProbeWidth::B1);
    }
    ctx
}

#[test]
#[should_panic(expected = "tick failed at cycle")]
fn harness_run_fails_the_test_when_a_tick_errors() {
    let mut ctx = context_heading_for_a_kernel_panic();
    ctx.run(2 * KERNEL_PANIC_GRACE_CYCLES);
}

#[test]
#[should_panic(expected = "tick failed at cycle")]
fn harness_run_until_fails_the_test_when_a_tick_errors() {
    let mut ctx = context_heading_for_a_kernel_panic();
    let _ = ctx.run_until(2 * KERNEL_PANIC_GRACE_CYCLES, |_| false);
}

#[test]
fn harness_x0_always_zero() {
    let mut ctx = TestContext::new();
    ctx.set_reg(X0, 999);
    assert_eq!(ctx.get_reg(X0), 0, "x0 must always read as zero");
}

#[test]
fn mock_memory_address_range() {
    use crate::soc::devices::Device;

    let mem = MockMemory::new(4096, 0x8000_0000);
    let region = mem.region();
    let (base, size) = (region.base.val(), region.size);
    assert_eq!(base, 0x8000_0000);
    assert_eq!(size, 4096);
}

#[test]
fn mock_memory_name() {
    use crate::soc::devices::Device;

    let mem = MockMemory::new(64, 0);
    assert_eq!(mem.name(), "Mock");
}
