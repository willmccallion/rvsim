//! SE mode's thread table: which hart a new thread takes, the order futex
//! wakes resume waiters in, timeouts, and how a thread's and a process's
//! exits free or stop harts.

use crate::common::{HartId, PhysAddr, VirtAddr};
use crate::config::{Config, Console};
use crate::isa::privileged::PagingMode;
use crate::soc::terminal::Terminal;
use crate::system::SystemState;
use crate::system::process::loader::UserContext;
use crate::system::process::{
    AddressSpace, FrameAllocator, Process, Processes, ThreadNotice, ThreadRef, Tid, WakeupKind,
};

const RAM_BASE: u64 = 0x8000_0000;
const FUTEX: VirtAddr = VirtAddr::new(0x1_0000);
const ANY: u32 = u32::MAX;
const ETIMEDOUT: u64 = 110;

/// A system of `harts` harts and a thread table whose one program's first
/// thread runs on hart 0.
fn table(harts: usize) -> (SystemState, Processes, ThreadRef) {
    let mut config = Config::minimal();
    config.system.hart_count = harts;
    let mut sys = SystemState::build(&config, "");
    let split = PhysAddr::new(RAM_BASE + (1 << 20));
    let mut frames = FrameAllocator::new(PhysAddr::new(RAM_BASE), split);
    let space = AddressSpace::new(PagingMode::Sv39, &mut sys.uncore.memory, &mut frames)
        .expect("the address space builds");
    let process = Process::new(space, VirtAddr::new(0x2_0000), 1, Terminal::shared(Console::Quiet));
    let mut processes =
        Processes::new(harts, FrameAllocator::new(split, PhysAddr::new(RAM_BASE + (2 << 20))));
    let tgid = processes.add_program(process, HartId::new(0));
    (sys, processes, ThreadRef { tgid, tid: tgid })
}

/// Starts a thread of `parent`'s process and lets its start take effect.
fn start_thread(sys: &SystemState, processes: &mut Processes, parent: ThreadRef) -> ThreadRef {
    let context = UserContext::of(&sys.harts[0], VirtAddr::new(0x1_0080));
    let tid = processes.spawn_thread(parent, context, None).expect("a hart is free");
    let _ = processes.take_wakeups();
    ThreadRef { tgid: parent.tgid, tid }
}

fn hart(index: u32) -> HartId {
    HartId::new(index)
}

/// The harts the wakeups taken now resume, in order, with their results.
fn resumed(processes: &mut Processes) -> Vec<(HartId, u64)> {
    processes
        .take_wakeups()
        .into_iter()
        .map(|wakeup| match wakeup.kind {
            WakeupKind::Resume { a0 } => (wakeup.hart, a0),
            WakeupKind::Start(_) => panic!("a start where a resume was expected"),
        })
        .collect()
}

#[test]
fn a_new_thread_takes_the_lowest_free_hart_and_the_next_id() {
    let (sys, mut processes, leader) = table(3);
    let context = UserContext::of(&sys.harts[0], VirtAddr::new(0x1_0080));

    let tid = processes.spawn_thread(leader, context, None);

    assert_eq!(tid, Some(Tid::new(2)));
    assert_eq!(
        processes.thread_on(hart(1)),
        Some(ThreadRef { tgid: leader.tgid, tid: Tid::new(2) })
    );
    let wakeups = processes.take_wakeups();
    assert_eq!(wakeups.len(), 1);
    assert_eq!(wakeups[0].hart, hart(1));
    assert!(matches!(wakeups[0].kind, WakeupKind::Start(_)));
    assert!(wakeups[0].flush_translations, "a hart's new thread drops its old translations");
}

#[test]
fn no_thread_starts_when_every_hart_is_taken() {
    let (sys, mut processes, leader) = table(1);
    let context = UserContext::of(&sys.harts[0], VirtAddr::new(0x1_0080));

    assert_eq!(processes.spawn_thread(leader, context, None), None);
}

#[test]
fn a_wake_resumes_the_oldest_waiters_first_up_to_its_count() {
    let (sys, mut processes, leader) = table(4);
    let threads: Vec<ThreadRef> =
        (0..3).map(|_| start_thread(&sys, &mut processes, leader)).collect();
    for &index in &[1, 0, 2] {
        processes.wait(threads[index], FUTEX, ANY, None);
    }

    let woken = processes.wake(leader.tgid, FUTEX, 2, ANY);

    assert_eq!(woken, 2);
    assert_eq!(resumed(&mut processes), vec![(hart(2), 0), (hart(1), 0)]);
}

#[test]
fn a_wake_reaches_only_waiters_whose_bitset_meets_its_own() {
    let (sys, mut processes, leader) = table(3);
    let first = start_thread(&sys, &mut processes, leader);
    let second = start_thread(&sys, &mut processes, leader);
    processes.wait(first, FUTEX, 0b01, None);
    processes.wait(second, FUTEX, 0b10, None);

    let woken = processes.wake(leader.tgid, FUTEX, usize::MAX, 0b10);

    assert_eq!(woken, 1);
    assert_eq!(resumed(&mut processes), vec![(hart(2), 0)]);
}

#[test]
fn a_wait_past_its_deadline_resumes_with_etimedout() {
    let (sys, mut processes, leader) = table(2);
    let thread = start_thread(&sys, &mut processes, leader);
    processes.wait(thread, FUTEX, ANY, Some(100));

    processes.expire_waits(99);
    let early = resumed(&mut processes);
    processes.expire_waits(100);

    assert!(early.is_empty());
    assert_eq!(resumed(&mut processes), vec![(hart(1), (-(ETIMEDOUT as i64)) as u64)]);
    assert_eq!(processes.next_deadline(), None);
}

/// Linux reports a process whose threads all called `exit` with its first
/// thread's code.
#[test]
fn the_last_thread_to_exit_ends_its_process_with_the_first_threads_code() {
    let (sys, mut processes, leader) = table(2);
    let thread = start_thread(&sys, &mut processes, leader);

    let after_leader = processes.exit_thread(leader, 3);
    let codes_meanwhile = processes.exit_codes();
    let after_last = processes.exit_thread(thread, 7);

    assert_eq!(after_leader, None);
    assert_eq!(codes_meanwhile, vec![None]);
    assert_eq!(after_last, Some(3));
    assert_eq!(processes.exit_codes(), vec![Some(3)]);
}

/// The caller's hart parks itself; a running thread's hart stops at its
/// next commit; a waiting thread's hart is parked already and free at once.
#[test]
fn exit_group_stops_running_threads_and_frees_waiting_ones() {
    let (sys, mut processes, leader) = table(3);
    let _running = start_thread(&sys, &mut processes, leader);
    let waiting = start_thread(&sys, &mut processes, leader);
    processes.wait(waiting, FUTEX, ANY, None);

    let run_exit = processes.exit_process(leader.tgid, 5, Some(leader.tid));

    assert_eq!(run_exit, Some(5));
    assert_eq!(processes.take_notice(hart(0)), None);
    assert_eq!(processes.take_notice(hart(1)), Some(ThreadNotice::Stop));
    assert_eq!(processes.take_notice(hart(1)), None);
    assert_eq!(processes.take_notice(hart(2)), None);
    assert_eq!((0..3).map(|h| processes.thread_on(hart(h))).collect::<Vec<_>>(), [None; 3]);
}

#[test]
fn a_start_queued_for_a_thread_whose_process_ended_is_dropped() {
    let (sys, mut processes, leader) = table(2);
    let context = UserContext::of(&sys.harts[0], VirtAddr::new(0x1_0080));
    let _ = processes.spawn_thread(leader, context, None);

    let _ = processes.exit_process(leader.tgid, 0, Some(leader.tid));

    assert!(processes.take_wakeups().is_empty());
}

/// After a sibling unmapped something, a running thread's hart drops its
/// translations at its next commit, once; a waiting one's when it resumes.
#[test]
fn stale_translations_are_dropped_at_commit_or_on_resume() {
    let (sys, mut processes, leader) = table(3);
    let _running = start_thread(&sys, &mut processes, leader);
    let waiting = start_thread(&sys, &mut processes, leader);
    processes.wait(waiting, FUTEX, ANY, None);

    processes.mark_translations_stale(leader);

    assert_eq!(processes.take_notice(hart(0)), None, "the caller already flushed its own");
    assert_eq!(processes.take_notice(hart(1)), Some(ThreadNotice::FlushTranslations));
    assert_eq!(processes.take_notice(hart(1)), None);
    assert_eq!(processes.take_notice(hart(2)), None, "a waiting thread's hart is parked");
    let _ = processes.wake(leader.tgid, FUTEX, 1, ANY);
    let wakeups = processes.take_wakeups();
    assert_eq!(wakeups.len(), 1);
    assert!(wakeups[0].flush_translations);
}

#[test]
fn a_checkpoint_keeps_waits_and_their_deadlines() {
    let (sys, mut processes, leader) = table(2);
    let thread = start_thread(&sys, &mut processes, leader);
    processes.wait(thread, FUTEX, ANY, Some(50));

    let mut restored =
        Processes::from_checkpoint(processes.checkpoint(), &Terminal::shared(Console::Quiet));
    restored.expire_waits(50);

    assert_eq!(resumed(&mut restored), vec![(hart(1), (-(ETIMEDOUT as i64)) as u64)]);
}
