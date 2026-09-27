//! Return Address Stack (RAS) tests: LIFO order, the circular overflow and
//! underflow of gem5's stack, and undoing squashed operations.

use rvsim_core::core::units::bru::ras::{Ras, RasHistory};

fn push(ras: &mut Ras, addr: u64) -> RasHistory {
    let mut history = RasHistory::default();
    ras.push(addr, &mut history);
    history
}

fn pop(ras: &mut Ras) -> Option<u64> {
    ras.pop(&mut RasHistory::default())
}

#[test]
fn pops_come_out_in_reverse_push_order() {
    let mut ras = Ras::new(8);
    let _ = push(&mut ras, 0xA);
    let _ = push(&mut ras, 0xB);
    let _ = push(&mut ras, 0xC);

    assert_eq!(pop(&mut ras), Some(0xC));
    assert_eq!(pop(&mut ras), Some(0xB));
    assert_eq!(pop(&mut ras), Some(0xA));
}

#[test]
fn top_reads_without_popping() {
    let mut ras = Ras::new(8);
    let _ = push(&mut ras, 0xAAAA);

    assert_eq!(ras.top(), Some(0xAAAA));
    assert_eq!(pop(&mut ras), Some(0xAAAA));
}

#[test]
fn a_pop_of_a_never_written_entry_predicts_nothing() {
    let mut ras = Ras::new(4);
    assert_eq!(pop(&mut ras), None);
}

#[test]
fn a_push_past_capacity_overwrites_the_oldest_entry() {
    let mut ras = Ras::new(2);
    let _ = push(&mut ras, 0x1);
    let _ = push(&mut ras, 0x2);
    let _ = push(&mut ras, 0x3);

    assert_eq!(pop(&mut ras), Some(0x3));
    assert_eq!(pop(&mut ras), Some(0x2));
    assert_eq!(pop(&mut ras), Some(0x3), "the stack wraps to the slot 0x3 overwrote");
}

#[test]
fn a_zero_capacity_stack_predicts_nothing() {
    let mut ras = Ras::new(0);
    let history = push(&mut ras, 0x1000);

    assert_eq!(pop(&mut ras), None);
    ras.squash(history);
}

#[test]
fn squashing_a_push_restores_the_previous_top() {
    let mut ras = Ras::new(8);
    let _ = push(&mut ras, 0x1000);
    let history = push(&mut ras, 0x2000);

    ras.squash(history);

    assert_eq!(ras.top(), Some(0x1000));
}

#[test]
fn squashing_a_pop_then_a_push_restores_the_overwritten_entry() {
    let mut ras = Ras::new(8);
    let _ = push(&mut ras, 0x1000);
    let _ = push(&mut ras, 0x2000);
    let mut popped = RasHistory::default();
    assert_eq!(ras.pop(&mut popped), Some(0x2000));
    let pushed = push(&mut ras, 0x3000);

    ras.squash(pushed);
    ras.squash(popped);

    assert_eq!(pop(&mut ras), Some(0x2000));
    assert_eq!(pop(&mut ras), Some(0x1000));
}
