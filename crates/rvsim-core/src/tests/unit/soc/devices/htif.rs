use crate::common::PhysAddr;
use crate::config::Console;
use crate::sim::memory::write_log::Writer;
use crate::sim::memory::{GlobalMemory, Ram};
use crate::soc::devices::{Device, Htif};
use crate::soc::exit::ExitSignal;
use crate::soc::terminal::{SharedTerminal, Terminal};
use crate::tests::support::probe;

fn quiet() -> SharedTerminal {
    Terminal::shared(Console::Quiet)
}

#[test]
fn reading_tohost_returns_zero() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal, Vec::new(), quiet());

    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 1)
            as u8,
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 2)
            as u16,
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 4)
            as u32,
        0
    );
    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1000), 8),
        0
    );
}

#[test]
fn byte_and_halfword_writes_to_tohost_are_ignored() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 1);
    assert_eq!(exit_signal.peek(), None);

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 2);
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn a_write_beside_tohost_does_not_end_the_run() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    crate::tests::support::probe::write(
        &mut htif,
        crate::common::PhysAddr::new(0x1000 + 4),
        1_u64,
        4,
    );
    assert_eq!(exit_signal.peek(), None);

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000 + 8), 1, 8);
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn a_tohost_value_of_one_passes_the_run() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1, 8);
    assert_eq!(exit_signal.peek(), Some(0));
}

#[test]
fn an_odd_tohost_value_above_one_fails_with_its_test_number() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    // value 3 is test number 1 (3 >> 1)
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 3, 8);
    assert_eq!(exit_signal.peek(), Some(1));

    // value 5 is test number 2 (5 >> 1)
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 5, 8);
    assert_eq!(exit_signal.peek(), Some(2));
}

#[test]
fn a_zero_tohost_value_is_ignored() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 0, 8);
    assert_eq!(exit_signal.peek(), None);
}

/// An even value is the address of a syscall block, not an exit; one
/// outside RAM is dropped and the run goes on.
#[test]
fn an_even_tohost_value_is_a_syscall_block_not_an_exit() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 42, 8);
    assert_eq!(exit_signal.peek(), None);
}

#[test]
fn a_word_write_to_tohost_counts() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    // U32 pass
    crate::tests::support::probe::write(&mut htif, crate::common::PhysAddr::new(0x1000), 1_u64, 4);
    assert_eq!(exit_signal.peek(), Some(0));
}

/// `fromhost` is part of a checkpoint: a restored device answers the
/// target's poll as the saved one would have.
#[test]
fn fromhost_survives_a_checkpoint() {
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal, Vec::new(), quiet());
    let state = serde_json::json!({ "fromhost": 5, "host_files_open": 0 });

    htif.restore(&state).expect("the state restores");

    assert_eq!(
        crate::tests::support::probe::read(&mut htif, crate::common::PhysAddr::new(0x1008), 8),
        5
    );
    assert_eq!(htif.checkpoint(), Some(state));
}

/// A checkpoint taken with host files open cannot be restored into a
/// device that has none of them.
#[test]
fn a_checkpoint_with_host_files_open_is_refused() {
    let exit_signal = ExitSignal::new();
    let htif = Htif::new(0x1000, exit_signal, Vec::new(), quiet());
    let state = serde_json::json!({ "fromhost": 0, "host_files_open": 2 });

    let refused = htif.check_restore(&state);

    assert!(refused.is_err(), "{refused:?}");
}

/// fesvr's `getcwd` is the host's, which fails with `ERANGE` when the
/// buffer cannot hold the path.
#[test]
fn getcwd_into_a_buffer_too_small_for_the_path_fails_with_erange() {
    const RAM_BASE: u64 = 0x8000_0000;
    const BLOCK: u64 = RAM_BASE + 0x100;
    const SYS_GETCWD: u64 = 17;
    const ERANGE: i64 = 34;
    let mut memory =
        GlobalMemory::new(Some(Ram::new(RAM_BASE, 0x1000).expect("test RAM maps")), 1, 64);
    let block: Vec<u8> = [SYS_GETCWD, RAM_BASE + 0x200, 1, 0, 0, 0, 0, 0]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .collect();
    memory.write_bytes(Writer::External, PhysAddr::new(BLOCK), &block);
    let mut htif = Htif::new(0x1000, ExitSignal::new(), Vec::new(), quiet());

    probe::write_and_run_dma(&mut htif, &mut memory, PhysAddr::new(0x1000), BLOCK, 8);

    let result = memory.read(PhysAddr::new(BLOCK), 8).expect("the block is in RAM");
    assert_eq!(result as i64, -ERANGE);
}

/// An exit code is any value the target passes, the largest included.
#[test]
fn an_exit_with_the_largest_code_still_ends_the_run() {
    const RAM_BASE: u64 = 0x8000_0000;
    const BLOCK: u64 = RAM_BASE + 0x100;
    const SYS_EXIT: u64 = 93;
    let mut memory =
        GlobalMemory::new(Some(Ram::new(RAM_BASE, 0x1000).expect("test RAM maps")), 1, 64);
    let block: Vec<u8> =
        [SYS_EXIT, u64::MAX, 0, 0, 0, 0, 0, 0].iter().flat_map(|w| w.to_le_bytes()).collect();
    memory.write_bytes(Writer::External, PhysAddr::new(BLOCK), &block);
    let exit_signal = ExitSignal::new();
    let mut htif = Htif::new(0x1000, exit_signal.clone(), Vec::new(), quiet());

    probe::write_and_run_dma(&mut htif, &mut memory, PhysAddr::new(0x1000), BLOCK, 8);

    assert_eq!(exit_signal.peek(), Some(u64::MAX));
}
