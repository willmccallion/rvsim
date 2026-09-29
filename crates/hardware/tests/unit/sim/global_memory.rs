//! A hart's access takes effect against the memory image when served.
//!
//! An AMO reads and writes in one step, and a store-conditional checks and
//! clears its hart's reservation as it writes.

use rvsim_core::common::{HartId, PhysAddr};
use rvsim_core::isa::op::AtomicOp;
use rvsim_core::sim::packet::{AccessSize, MemOp, MemRespData};
use rvsim_core::soc::memory::RamRegion;
use rvsim_core::system::state::global_memory::GlobalMemory;

const H0: HartId = HartId::new(0);
const H1: HartId = HartId::new(1);
const RAM_BYTES: usize = 0x1000;
const WORD: PhysAddr = PhysAddr::new(0x100);

/// Two harts' memory over a RAM buffer the caller keeps alive.
fn memory(ram: &mut [u8]) -> GlobalMemory {
    let region = RamRegion::new(ram.as_mut_ptr(), 0, ram.len() as u64);
    GlobalMemory::new(Some(region), 2, 64)
}

fn value(response: &MemRespData) -> u64 {
    match response {
        MemRespData::Performed { value, .. } => *value,
        other => panic!("expected a performed access, got {other:?}"),
    }
}

fn amo(op: AtomicOp, data: u64, hart: HartId) -> MemOp {
    MemOp::Atomic { op, data, hart }
}

#[test]
fn an_amo_returns_the_old_value_and_leaves_the_result() {
    let mut ram = vec![0u8; RAM_BYTES];
    let mut memory = memory(&mut ram);
    let _ = memory.perform(WORD, AccessSize::B8, &amo(AtomicOp::Swap, 5, H0));

    let old = memory.perform(WORD, AccessSize::B8, &amo(AtomicOp::Add, 3, H1));

    assert_eq!(value(&old), 5);
    assert_eq!(memory.read(WORD, 8), Some(8));
}

#[test]
fn a_word_amo_writes_only_its_word() {
    let mut ram = vec![0xFFu8; RAM_BYTES];
    let mut memory = memory(&mut ram);

    let _ = memory.perform(WORD, AccessSize::B4, &amo(AtomicOp::Swap, 0, H0));

    assert_eq!(memory.read(WORD, 8), Some(0xFFFF_FFFF_0000_0000));
}

#[test]
fn a_store_conditional_with_its_reservation_writes_and_returns_zero() {
    let mut ram = vec![0u8; RAM_BYTES];
    let mut memory = memory(&mut ram);
    memory.reservations_mut().set(H0, WORD);

    let result = memory.perform(WORD, AccessSize::B8, &amo(AtomicOp::Sc, 9, H0));

    assert_eq!(value(&result), 0);
    assert_eq!(memory.read(WORD, 8), Some(9));
    assert!(!memory.reservations().check(H0, WORD), "an SC clears its reservation");
}

#[test]
fn a_store_conditional_whose_reservation_another_hart_broke_fails() {
    let mut ram = vec![0u8; RAM_BYTES];
    let mut memory = memory(&mut ram);
    memory.reservations_mut().set(H0, WORD);
    let _ = memory.perform(WORD, AccessSize::B8, &amo(AtomicOp::Swap, 1, H1));

    let result = memory.perform(WORD, AccessSize::B8, &amo(AtomicOp::Sc, 9, H0));

    assert_eq!(value(&result), 1);
    assert_eq!(memory.read(WORD, 8), Some(1));
}

#[test]
fn a_span_read_returns_its_bytes_in_address_order() {
    let mut ram = vec![0u8; RAM_BYTES];
    for (i, byte) in ram[0x200..0x220].iter_mut().enumerate() {
        *byte = i as u8 + 1;
    }
    let mut memory = memory(&mut ram);

    let response = memory.perform(PhysAddr::new(0x208), AccessSize::Span(24), &MemOp::Read);

    let MemRespData::PerformedBytes { bytes, observed } = response else {
        panic!("expected the span's bytes, got {response:?}");
    };
    assert_eq!(&*bytes, &(9..=32).collect::<Vec<u8>>()[..]);
    assert!(observed.is_some(), "a two-hart read carries its write order");
}

#[test]
fn a_span_read_takes_effect_where_it_is_served() {
    assert!(MemOp::Read.takes_effect_when_served(AccessSize::Span(64)));
    assert!(!MemOp::Read.takes_effect_when_served(AccessSize::Line));
}
