//! The commit log a lockstep checker replays carries each retired
//! instruction's register, CSR and memory effects and each trap, the same
//! on every backend.

use crate::tests::support::regs::{T0, T1, X0, X7, X8, X9, X10, X11, X12, X13, X14};
use std::collections::HashMap;

use crate::common::{PhysAddr, VirtAddr};
use crate::config::{BackendKind, Config};
use crate::exec::compute::vector::mem::ElementAccess;
use crate::isa::op::{MemWidth, SystemOp};
use crate::isa::privileged::Trap;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::{ElemIdx, Sew};
use crate::system::simulator::ProbeWidth;
use crate::tests::support::builder::instruction::{ECALL, InstructionBuilder};
use crate::tests::support::harness::{PROGRAM_BASE, TestContext};
use crate::uarch::pipeline::commit_log::{MemEffect, RESET_CSRS, VectorElementAccess};
use crate::uarch::pipeline::latches::{
    Mem1Mem2Entry, MicroOpIdx, VecMemAccess, VecMemSpan, VecMemTarget,
};

const DATA: u64 = PROGRAM_BASE + 0x1000;
const HANDLER: u64 = PROGRAM_BASE + 0x50;
const VALUE: u64 = 0x7f;
const MSCRATCH: u32 = 0x340;
const MSTATUS: u32 = 0x300;
const MTVEC: u32 = 0x305;
const ECALL_FROM_M: u64 = 11;

const FFLAGS_DZ: u64 = 0x08;

/// `fmv.d.x f1, t1`.
const FMV_D_X_F1_T1: u32 = 0xf200_0053 | (T1.field() << 15) | (1 << 7);
/// `fdiv.d f2, f1, f0` rounding to nearest: a division by zero.
const FDIV_D_F2_F1_F0: u32 = 0x1a00_0053 | (1 << 15) | (2 << 7);
/// `vsetvli a5, zero, e64, m1`.
const VSETVLI_A5_E64: u32 = 0x57 | (15 << 7) | (0b111 << 12) | (0x18 << 20);

/// Stores, loads, an AMO, an LR/SC pair, CSR writes, FP and vector
/// instructions, then an ECALL into a handler at `HANDLER` that spins.
fn every_effect() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T0, 1).build(),
        i().addi(T1, X0, VALUE as i32).build(),
        i().sd(T0, T1, 0).build(),
        i().ld(X7, T0, 0).build(),
        i().sb(T0, T1, 9).build(),
        i().lbu(X8, T0, 9).build(),
        i().amoadd_d(X9, T0, T1).build(),
        i().lr_d(X10, T0).build(),
        i().sc_d(X11, T0, T1).build(),
        i().csrrw(X12, MSCRATCH, T1).build(),
        i().lui(X13, 0x6).build(),
        i().addi(X13, X13, 0x600).build(),
        i().csrrs(X0, MSTATUS, X13).build(),
        FMV_D_X_F1_T1,
        FDIV_D_F2_F1_F0,
        VSETVLI_A5_E64,
        i().auipc(X14, 0).build(),
        i().addi(X14, X14, 16).build(),
        i().csrrw(X0, MTVEC, X14).build(),
        ECALL,
        i().jal(X0, 0).build(),
    ]
}

/// Runs `every_effect` on `backend` with the commit log open and returns
/// the log's lines.
fn logged_lines(backend: BackendKind, width: usize) -> Vec<String> {
    logged_lines_until(&every_effect(), HANDLER, backend, width)
}

/// Runs `program` on `backend` with the commit log open until the PC
/// reaches `stop_pc`, and returns the log's lines.
fn logged_lines_until(
    program: &[u32],
    stop_pc: u64,
    backend: BackendKind,
    width: usize,
) -> Vec<String> {
    let mut config = Config::minimal();
    config.pipeline.backend = backend;
    config.pipeline.width = width;
    let log = tempfile::NamedTempFile::new().expect("temp file");
    let path = log.path().to_str().expect("utf-8 path").to_owned();
    let mut ctx = TestContext::with_fast_memory(&config).load_program(PROGRAM_BASE, program);
    ctx.sim.open_commit_log(&path).expect("open commit log");

    let reached = ctx.run_until(5_000, |ctx| ctx.state().harts[0].pc == VirtAddr::new(stop_pc));
    ctx.run(20);
    drop(ctx);

    assert!(reached.is_some(), "{backend:?} width {width}: the program never reached {stop_pc:#x}");
    std::fs::read_to_string(&path).expect("read commit log").lines().map(str::to_owned).collect()
}

/// An instruction's line taken apart: its PC, the cycle it retired in and
/// the effects after its header.
struct Logged {
    pc: u64,
    cycle: u64,
    effects: String,
}

/// `line` taken apart when it is an instruction's line retired in M-mode.
fn parse_instruction(line: &str) -> Option<Logged> {
    let rest = line.strip_prefix("core   0: 0x")?;
    let pc = u64::from_str_radix(&rest[..16], 16).ok()?;
    let after_header = rest.split_once(" priv 3 cycle ")?.1;
    let (cycle, effects) = after_header.split_once(' ').unwrap_or((after_header, ""));
    Some(Logged { pc, cycle: cycle.parse().ok()?, effects: effects.trim().to_owned() })
}

/// The instruction lines of `lines`, in log order.
fn logged_instructions(lines: &[String]) -> Vec<Logged> {
    lines.iter().filter_map(|line| parse_instruction(line)).collect()
}

/// The effects logged after each instruction's header, by PC.
fn effects_by_pc(lines: &[String]) -> HashMap<u64, String> {
    logged_instructions(lines).into_iter().map(|logged| (logged.pc, logged.effects)).collect()
}

fn mem(kind: &str, addr: u64, bytes: u64, value: u64) -> String {
    format!("{kind} 0x{addr:016x} 0x{addr:016x} {bytes} 0x{value:016x}")
}

#[test]
fn each_memory_access_is_logged_with_its_address_bytes_and_value() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));
    let at = |index: u64| effects[&(PROGRAM_BASE + 4 * index)].clone();

    assert_eq!(at(2), mem("store", DATA, 8, VALUE));
    assert_eq!(at(3), format!("x7 0x{VALUE:016x} {}", mem("load", DATA, 8, VALUE)));
    assert_eq!(at(4), mem("store", DATA + 9, 1, VALUE));
    assert_eq!(at(5), format!("x8 0x{VALUE:016x} {}", mem("load", DATA + 9, 1, VALUE)));
    assert_eq!(
        at(6),
        format!(
            "x9 0x{VALUE:016x} {} {}",
            mem("load", DATA, 8, VALUE),
            mem("store", DATA, 8, 2 * VALUE)
        )
    );
    assert_eq!(at(7), format!("x10 0x{:016x} {}", 2 * VALUE, mem("load", DATA, 8, 2 * VALUE)));
    assert_eq!(at(8), format!("x11 0x{:016x} {}", 0, mem("store", DATA, 8, VALUE)));
}

#[test]
fn a_csr_write_logs_the_value_read_back_after_it() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));

    assert_eq!(effects[&(PROGRAM_BASE + 4 * 9)], format!("x12 0x{:016x} c340 0x{VALUE:016x}", 0));
    assert!(
        effects[&(PROGRAM_BASE + 4 * 12)].starts_with("c300 0x"),
        "the mstatus write logged {:?}",
        effects[&(PROGRAM_BASE + 4 * 12)]
    );
}

#[test]
fn an_fp_destination_is_logged_as_an_f_register() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));

    assert_eq!(effects[&(PROGRAM_BASE + 4 * 13)], format!("f1 0x{VALUE:016x}"));
}

#[test]
fn raised_fp_flags_are_logged_as_the_fflags_they_leave() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));

    assert_eq!(
        effects[&(PROGRAM_BASE + 4 * 14)],
        format!("f2 0x{:016x} c001 0x{FFLAGS_DZ:016x}", f64::INFINITY.to_bits())
    );
}

#[test]
fn a_vector_instruction_is_marked_and_keeps_its_scalar_result() {
    let effects = effects_by_pc(&logged_lines(BackendKind::InOrder, 1));
    let vsetvli = &effects[&(PROGRAM_BASE + 4 * 15)];

    assert!(
        vsetvli.starts_with("x15 0x") && vsetvli.ends_with(" vec"),
        "vsetvli logged {vsetvli:?}"
    );
}

#[test]
fn the_log_opens_with_the_state_it_starts_from() {
    let lines = logged_lines(BackendKind::InOrder, 1);
    let reset_csrs: Vec<String> = lines
        .iter()
        .filter_map(|line| line.strip_prefix("core   0: reset c"))
        .map(|rest| rest[..3].to_owned())
        .collect();
    let expected: Vec<String> =
        RESET_CSRS.iter().map(|addr| format!("{:03x}", addr.as_u32())).collect();

    assert_eq!(lines[0], format!("core   0: reset pc 0x{PROGRAM_BASE:016x} priv 3"));
    assert_eq!(reset_csrs, expected);
}

#[test]
fn each_instruction_logs_the_cycle_it_retired_in() {
    let logged = logged_instructions(&logged_lines(BackendKind::InOrder, 1));
    let cycles: Vec<u64> = logged.iter().map(|entry| entry.cycle).collect();

    assert!(cycles.iter().all(|&cycle| cycle > 0), "retired before the first cycle: {cycles:?}");
    assert!(cycles.windows(2).all(|pair| pair[0] <= pair[1]), "retire cycles go back: {cycles:?}");
}

#[test]
fn a_trap_is_logged_after_the_instruction_that_raised_it() {
    let lines = logged_lines(BackendKind::InOrder, 1);
    let ecall_pc = PROGRAM_BASE + 4 * 19;
    let ecall = lines
        .iter()
        .position(|line| line.starts_with(&format!("core   0: 0x{ecall_pc:016x} ")))
        .expect("the ECALL is logged");
    let retired = parse_instruction(&lines[ecall]).expect("an instruction line").cycle;
    let trap =
        format!("core   0: trap 0x{ECALL_FROM_M:016x} 0x{ecall_pc:016x} 0x{:016x} cycle ", 0);

    assert_eq!(
        lines[ecall],
        format!("core   0: 0x{ecall_pc:016x} (0x{ECALL:08x}) priv 3 cycle {retired}")
    );
    let taken: u64 = lines[ecall + 1]
        .strip_prefix(&trap)
        .and_then(|cycle| cycle.parse().ok())
        .unwrap_or_else(|| panic!("the trap line is {:?}", lines[ecall + 1]));
    assert!(
        taken >= retired,
        "the trap was taken in {taken}, before the ECALL retired in {retired}"
    );
}

/// `line` without its `cycle <n>` field, so logs of different timing compare.
fn without_cycle(line: &str) -> String {
    let Some((head, tail)) = line.split_once(" cycle ") else { return line.to_owned() };
    match tail.split_once(' ') {
        Some((_, effects)) => format!("{head} {effects}"),
        None => head.to_owned(),
    }
}

/// The lines up to the handler's first instruction without their cycles;
/// how often the handler spins after it depends on timing.
fn up_to_handler(lines: &[String]) -> Vec<String> {
    let handler = format!("core   0: 0x{HANDLER:016x} ");
    let end = lines.iter().position(|line| line.starts_with(&handler)).expect("the handler ran");
    lines[..=end].iter().map(|line| without_cycle(line)).collect()
}

#[test]
fn every_backend_logs_the_same_retirements() {
    let reference = up_to_handler(&logged_lines(BackendKind::InOrder, 1));

    for (backend, width) in
        [(BackendKind::InOrder, 4), (BackendKind::OutOfOrder, 1), (BackendKind::OutOfOrder, 4)]
    {
        let lines = up_to_handler(&logged_lines(backend, width));

        assert_eq!(lines, reference, "{backend:?} width {width}");
    }
}

const MSTATUS_VS_INITIAL: i32 = 0x600;
/// `vmv.v.i v<vd>, 5`.
const fn vmv_v_i_5(vd: u32) -> u32 {
    (0b01_0111 << 26) | (1 << 25) | (5 << 15) | (0b011 << 12) | (vd << 7) | 0x57
}

/// Enables V, sets vl = 1 with a tail-agnostic e64 vtype, writes 5 to v1's
/// body, then spins.
fn vector_tail() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().addi(X13, X0, MSTATUS_VS_INITIAL).build(),
        i().csrrs(X0, MSTATUS, X13).build(),
        vsetivli(1, VTYPE_E64_M1_TA_MA),
        vmv_v_i_5(1),
        i().jal(X0, 0).build(),
    ]
}

#[test]
fn a_vector_write_logs_the_register_and_the_bits_it_filled_agnostically() {
    let spin = PROGRAM_BASE + 4 * 4;
    let tail_ones_body_five = format!("0x{:016x}{:016x}", u64::MAX, 5);
    let tail_agnostic = format!("0x{:016x}{:016x}", u64::MAX, 0);

    for (backend, width) in [(BackendKind::InOrder, 1), (BackendKind::OutOfOrder, 4)] {
        let effects = effects_by_pc(&logged_lines_until(&vector_tail(), spin, backend, width));

        assert_eq!(
            effects[&(PROGRAM_BASE + 4 * 3)],
            format!("v1 {tail_ones_body_five} {tail_agnostic} vec"),
            "{backend:?}"
        );
    }
}

const VECTOR_LOAD_OPCODE: u32 = 0x07;
const VECTOR_STORE_OPCODE: u32 = 0x27;
const FAULT_ONLY_FIRST: u32 = 0b1_0000 << 20;
const UNMASKED: u32 = 1 << 25;
const ELEMENT_BYTES: u64 = 8;
const FIRST_ELEMENT: u64 = 0x11;
const SECOND_ELEMENT: u64 = 0x22;
const VECTOR_STORE_OFFSET: i32 = 0x100;

/// The vtype of 64-bit elements, tail- and mask-agnostic, at LMUL 1 and 2.
const VTYPE_E64_M1_TA_MA: u32 = 0xd8;
const VTYPE_E64_M2_TA_MA: u32 = 0xd9;

/// `vsetivli a5, vl, <vtype>`.
const fn vsetivli(vl: u32, vtype: u32) -> u32 {
    (0b11 << 30) | (vtype << 20) | (vl << 15) | (0b111 << 12) | (15 << 7) | 0x57
}

/// `vle64.v`/`vse64.v` of `v1` at the address in `base`, per `opcode`.
const fn vector_move_e64(opcode: u32, base: RegIdx) -> u32 {
    opcode | UNMASKED | (base.field() << 15) | (0b111 << 12) | (1 << 7)
}

fn vector_access(kind: &str, index: u64, address: u64, value: u64) -> String {
    mem(kind, address + ELEMENT_BYTES * index, ELEMENT_BYTES, value)
}

/// Stores two elements at `DATA`, loads them into `v1` and stores `v1` at
/// `DATA + VECTOR_STORE_OFFSET`, then spins.
fn vector_copy() -> Vec<u32> {
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T0, 1).build(),
        i().addi(T1, X0, FIRST_ELEMENT as i32).build(),
        i().addi(X8, X0, SECOND_ELEMENT as i32).build(),
        i().sd(T0, T1, 0).build(),
        i().sd(T0, X8, ELEMENT_BYTES as i32).build(),
        i().addi(X7, T0, VECTOR_STORE_OFFSET).build(),
        vsetivli(2, VTYPE_E64_M1_TA_MA),
        vector_move_e64(VECTOR_LOAD_OPCODE, T0),
        vector_move_e64(VECTOR_STORE_OPCODE, X7),
        i().jal(X0, 0).build(),
    ]
}

#[test]
fn a_vector_load_and_store_log_each_element_in_element_order() {
    let target = DATA + VECTOR_STORE_OFFSET as u64;
    let spin = PROGRAM_BASE + 4 * 9;
    let loaded = format!(
        "{} {}",
        vector_access("vload", 0, DATA, FIRST_ELEMENT),
        vector_access("vload", 1, DATA, SECOND_ELEMENT)
    );
    let stored = format!(
        "{} {} vec",
        vector_access("vstore", 0, target, FIRST_ELEMENT),
        vector_access("vstore", 1, target, SECOND_ELEMENT)
    );
    let vector_value = format!("0x{SECOND_ELEMENT:016x}{FIRST_ELEMENT:016x} 0x{:032x}", 0);

    for (backend, width) in [(BackendKind::InOrder, 1), (BackendKind::OutOfOrder, 4)] {
        let effects = effects_by_pc(&logged_lines_until(&vector_copy(), spin, backend, width));

        assert_eq!(
            effects[&(PROGRAM_BASE + 4 * 7)],
            format!("v1 {vector_value} {loaded} vec"),
            "{backend:?} vle64.v"
        );
        assert_eq!(effects[&(PROGRAM_BASE + 4 * 8)], stored, "{backend:?} vse64.v");
    }
}

/// Loads two elements fault-only-first from the last doubleword of RAM, so
/// the second lies outside it.
fn fault_only_first_at_the_end_of_ram() -> Vec<u32> {
    let ram_pages = i32::try_from(Config::minimal().memory.ram_size >> 12).expect("small RAM");
    let i = InstructionBuilder::new;
    vec![
        i().auipc(T0, ram_pages).build(),
        i().addi(T0, T0, -(ELEMENT_BYTES as i32)).build(),
        i().addi(T1, X0, FIRST_ELEMENT as i32).build(),
        i().sd(T0, T1, 0).build(),
        vsetivli(2, VTYPE_E64_M1_TA_MA),
        vector_move_e64(VECTOR_LOAD_OPCODE, T0) | FAULT_ONLY_FIRST,
        i().jal(X0, 0).build(),
    ]
}

#[test]
fn a_fault_only_first_load_logs_only_the_elements_before_the_fault() {
    let last_element = PROGRAM_BASE + Config::minimal().memory.ram_size - ELEMENT_BYTES;
    let load_pc = PROGRAM_BASE + 4 * 5;
    let spin = PROGRAM_BASE + 4 * 6;
    let first_load = vector_access("vload", 0, last_element, FIRST_ELEMENT);

    for (backend, width) in [(BackendKind::InOrder, 1), (BackendKind::OutOfOrder, 4)] {
        let lines = logged_lines_until(&fault_only_first_at_the_end_of_ram(), spin, backend, width);
        let load = &effects_by_pc(&lines)[&load_pc];

        assert!(load.ends_with(&format!("{first_load} vec")), "{backend:?} logged {load:?}");
        assert_eq!(load.matches("vload").count(), 1, "{backend:?} logged {load:?}");
    }
}

#[test]
fn a_vector_group_write_logs_each_register_of_the_group() {
    let program = [
        vsetivli(4, VTYPE_E64_M2_TA_MA),
        vmv_v_i_5(2),
        InstructionBuilder::new().jal(X0, 0).build(),
    ];
    let spin = PROGRAM_BASE + 4 * 2;
    let body_five = format!("0x{:016x}{:016x} 0x{:032x}", 5, 5, 0);

    for (backend, width) in [(BackendKind::InOrder, 1), (BackendKind::OutOfOrder, 4)] {
        let effects = effects_by_pc(&logged_lines_until(&program, spin, backend, width));

        assert_eq!(
            effects[&(PROGRAM_BASE + 4)],
            format!("v2 {body_five} v3 {body_five} vec"),
            "{backend:?}"
        );
    }
}

#[test]
fn memory_the_host_writes_is_logged_before_the_load_that_reads_it() {
    const HOST_VALUE: u64 = 0x0123_4567_89ab_cdef;
    let i = InstructionBuilder::new;
    let program = [i().auipc(T0, 1).build(), i().ld(X7, T0, 0).build(), i().jal(X0, 0).build()];
    let log = tempfile::NamedTempFile::new().expect("temp file");
    let path = log.path().to_str().expect("utf-8 path").to_owned();
    let mut ctx =
        TestContext::with_fast_memory(&Config::minimal()).load_program(PROGRAM_BASE, &program);
    ctx.sim.open_commit_log(&path).expect("open commit log");

    ctx.sim.probe_mem_store(PhysAddr::new(DATA + 4), HOST_VALUE, ProbeWidth::B8);
    ctx.run(200);
    drop(ctx);

    let lines: Vec<String> = std::fs::read_to_string(&path)
        .expect("read commit log")
        .lines()
        .map(str::to_owned)
        .collect();
    let host_write = format!("core   0: host write 0x{:016x} 0xefcdab8967452301", DATA + 4);
    let written =
        lines.iter().position(|line| *line == host_write).expect("the host write is logged");
    let load = lines.iter().position(|line| line.contains(" load ")).expect("the load is logged");
    assert!(written < load, "{lines:#?}");
}

const ACCESS_PADDR: u64 = 0x8000_2000;

/// A doubleword load leaving memory2 with `VALUE` read from `DATA`.
fn load_leaving_memory2() -> Mem1Mem2Entry {
    let mut mem = Mem1Mem2Entry::default();
    mem.op.ctrl.mem_read = true;
    mem.op.ctrl.width = MemWidth::Double;
    mem.vaddr = VirtAddr::new(DATA);
    mem.paddr = PhysAddr::new(ACCESS_PADDR);
    mem.load_data = VALUE;
    mem
}

#[test]
fn a_load_leaving_memory2_logs_the_value_it_read() {
    let effect = MemEffect::of_memory2(&load_leaving_memory2());

    assert_eq!(
        effect,
        Some(MemEffect::new(
            VirtAddr::new(DATA),
            PhysAddr::new(ACCESS_PADDR),
            MemWidth::Double,
            Some(VALUE),
            None
        ))
    );
}

#[test]
fn an_access_keeps_only_the_bytes_of_its_width() {
    const ALL_ONES_BELOW_WIDTH: [(MemWidth, u64); 4] = [
        (MemWidth::Byte, 0xff),
        (MemWidth::Half, 0xffff),
        (MemWidth::Word, 0xffff_ffff),
        (MemWidth::Double, u64::MAX),
    ];

    for (width, kept) in ALL_ONES_BELOW_WIDTH {
        let effect = MemEffect::new(
            VirtAddr::new(DATA),
            PhysAddr::new(ACCESS_PADDR),
            width,
            Some(u64::MAX),
            Some(u64::MAX),
        );

        assert_eq!((effect.read, effect.write), (Some(kept), Some(kept)), "{width:?}");
    }
}

#[test]
fn a_faulting_load_has_no_memory_effect() {
    let mut mem = load_leaving_memory2();
    mem.op.trap = Some(Trap::LoadAccessFault(DATA));

    assert_eq!(MemEffect::of_memory2(&mem), None);
}

#[test]
fn a_vector_memory_micro_op_has_no_scalar_memory_effect() {
    let mut mem = load_leaving_memory2();
    mem.op.vec_mem = Some(VecMemAccess {
        micro_op: MicroOpIdx::new(0),
        is_store: false,
        target: VecMemTarget::Element { elem_idx: ElemIdx::new(0), eew: Sew::E64, dest_slot: 0 },
    });

    assert_eq!(MemEffect::of_memory2(&mem), None);
}

#[test]
fn a_cache_block_operation_has_no_memory_effect() {
    let mut mem = load_leaving_memory2();
    mem.op.ctrl.system_op = SystemOp::CboFlush;

    assert_eq!(MemEffect::of_memory2(&mem), None);
}

const HALF_BYTES: u64 = 2;
const FIRST_HALF: u64 = 0x1234;
const SECOND_HALF: u64 = 0x5678;

fn half_element(index: usize, store_data: u64) -> (MicroOpIdx, ElementAccess) {
    let access = ElementAccess {
        vaddr: VirtAddr::new(DATA + HALF_BYTES * index as u64),
        store_data,
        elem_idx: ElemIdx::new(index),
        eew: Sew::E16,
        dest_slot: 0,
    };
    (MicroOpIdx::new(index), access)
}

/// A memory2 entry for a vector micro-op over two halfword elements.
fn span_leaving_memory2(is_store: bool, data: Option<Box<[u8]>>) -> Mem1Mem2Entry {
    let span = VecMemSpan {
        elements: vec![half_element(0, FIRST_HALF), half_element(1, SECOND_HALF)],
        data,
    };
    let mut mem = Mem1Mem2Entry::default();
    mem.op.vec_mem = Some(VecMemAccess {
        micro_op: MicroOpIdx::new(0),
        is_store,
        target: VecMemTarget::Span(Box::new(span)),
    });
    mem.paddr = PhysAddr::new(ACCESS_PADDR);
    mem
}

fn half_effect(index: u64, read: Option<u64>, write: Option<u64>) -> MemEffect {
    MemEffect::new(
        VirtAddr::new(DATA + HALF_BYTES * index),
        PhysAddr::new(ACCESS_PADDR + HALF_BYTES * index),
        MemWidth::Half,
        read,
        write,
    )
}

#[test]
fn a_vector_load_span_logs_each_element_it_read_in_memory_order() {
    let bytes = [0x34, 0x12, 0x78, 0x56];
    let mem = span_leaving_memory2(false, Some(Box::from(bytes)));

    let accesses = VectorElementAccess::of_memory2(&mem);

    assert_eq!(
        accesses,
        [
            VectorElementAccess {
                element: ElemIdx::new(0),
                effect: half_effect(0, Some(FIRST_HALF), None)
            },
            VectorElementAccess {
                element: ElemIdx::new(1),
                effect: half_effect(1, Some(SECOND_HALF), None)
            },
        ]
    );
}

#[test]
fn a_vector_store_span_logs_each_element_it_wrote() {
    let mem = span_leaving_memory2(true, None);

    let accesses = VectorElementAccess::of_memory2(&mem);

    assert_eq!(
        accesses.iter().map(|access| access.effect).collect::<Vec<_>>(),
        [half_effect(0, None, Some(FIRST_HALF)), half_effect(1, None, Some(SECOND_HALF))]
    );
}

#[test]
fn a_single_vector_element_logs_the_value_its_micro_op_moved() {
    let mut mem = load_leaving_memory2();
    mem.op.vec_mem = Some(VecMemAccess {
        micro_op: MicroOpIdx::new(0),
        is_store: true,
        target: VecMemTarget::Element { elem_idx: ElemIdx::new(3), eew: Sew::E64, dest_slot: 0 },
    });
    mem.store_data = SECOND_HALF;

    let accesses = VectorElementAccess::of_memory2(&mem);

    let effect = MemEffect::new(
        VirtAddr::new(DATA),
        PhysAddr::new(ACCESS_PADDR),
        MemWidth::Double,
        None,
        Some(SECOND_HALF),
    );
    assert_eq!(accesses, [VectorElementAccess { element: ElemIdx::new(3), effect }]);
}

#[test]
fn a_faulting_vector_element_logs_no_access() {
    let mut mem = span_leaving_memory2(false, None);
    mem.op.trap = Some(Trap::LoadAccessFault(DATA));

    assert!(VectorElementAccess::of_memory2(&mem).is_empty());
}

const RESET_INT_REG: u8 = 5;
const RESET_LAST_INT_REG: u8 = 31;
const RESET_FP_REG: u8 = 3;
const RESET_FIRST_FP_REG: u8 = 0;
const RESET_VALUE: u64 = 0x1234_5678_9abc_def0;

#[test]
fn the_reset_lines_list_exactly_the_nonzero_registers() {
    let log = tempfile::NamedTempFile::new().expect("temp file");
    let path = log.path().to_str().expect("utf-8 path").to_owned();
    let mut ctx = TestContext::with_fast_memory(&Config::minimal());
    ctx.set_reg(RegIdx::new(RESET_INT_REG), RESET_VALUE);
    ctx.set_reg(RegIdx::new(RESET_LAST_INT_REG), RESET_VALUE);
    ctx.state_mut().harts[0].regs.write_f(RegIdx::new(RESET_FP_REG), RESET_VALUE);
    ctx.state_mut().harts[0].regs.write_f(RegIdx::new(RESET_FIRST_FP_REG), RESET_VALUE);

    ctx.sim.open_commit_log(&path).expect("open commit log");
    drop(ctx);

    let lines = std::fs::read_to_string(&path).expect("read commit log");
    let registers: Vec<&str> = lines
        .lines()
        .filter(|line| {
            line.starts_with("core   0: reset x") || line.starts_with("core   0: reset f")
        })
        .collect();
    let expected = [
        format!("core   0: reset x{RESET_INT_REG} 0x{RESET_VALUE:016x}"),
        format!("core   0: reset x{RESET_LAST_INT_REG} 0x{RESET_VALUE:016x}"),
        format!("core   0: reset f{RESET_FIRST_FP_REG} 0x{RESET_VALUE:016x}"),
        format!("core   0: reset f{RESET_FP_REG} 0x{RESET_VALUE:016x}"),
    ];
    assert_eq!(registers, expected);
}
