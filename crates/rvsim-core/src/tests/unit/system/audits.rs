//! The cache and coherence audits, shown to report each invariant they
//! check: every test plants one fault in an otherwise empty system.

use crate::Simulator;
use crate::common::{CoreId, LineAddr, PhysAddr};
use crate::config::{Config, Console, InclusionPolicy};
use crate::sim::components::{ComponentId, ReqId};
use crate::sim::packet::{AccessSize, MemOp, MesiState};
use crate::soc::cache::mshr::{Mshr, MshrTarget};
use crate::soc::cache::writeback_buffer::{Writeback, WritebackCause};
use crate::soc::cache::{BookkeepingFault, Cache};
use crate::soc::coherence::protocol::CoreSet;
use crate::system::coherence_audit::{InclusionFault, ViolationKind};

/// The address every test plants its fault at.
const ADDR: u64 = 0x8000_1000;

const CORE_0: CoreId = CoreId::new(0);
const CORE_1: CoreId = CoreId::new(1);

/// The desktop machine with `hart_count` cores and nothing cached yet.
fn machine(hart_count: usize, inclusion_policy: InclusionPolicy) -> Simulator {
    let mut config = Config::default();
    config.system.hart_count = hart_count;
    config.system.console = Console::Quiet;
    config.cache.inclusion_policy = inclusion_policy;
    Simulator::build(&config, "")
}

fn two_cores() -> Simulator {
    machine(2, InclusionPolicy::default())
}

fn l1d(sim: &mut Simulator, core: CoreId) -> &mut Cache {
    &mut sim.state.cores[core.as_index()].units.l1_d_cache
}

fn l2(sim: &mut Simulator, core: CoreId) -> &mut Cache {
    &mut sim.state.cores[core.as_index()].units.l2_cache
}

/// Plants `ADDR`'s line in `cache`, in its first way.
fn hold(cache: &mut Cache, state: MesiState) {
    cache.plant_line(0, ADDR, state);
}

/// Records in the home agent that `core` was granted `ADDR` in `state`.
fn grant(sim: &mut Simulator, core: CoreId, state: MesiState) {
    let line = line_of(sim, ADDR);
    let fabric = sim.state.uncore.coherence.as_mut().expect("two cores have a coherence fabric");
    fabric.plant_grant(line, core, state);
}

fn line_of(sim: &Simulator, addr: u64) -> LineAddr {
    let line_bytes = sim.state.cores[0].units.l1_d_cache.line_bytes() as u64;
    LineAddr::from_phys(PhysAddr::new(addr), line_bytes)
}

/// The kinds of `violations`, each checked to concern `ADDR`'s line.
fn kinds_at_addr(
    sim: &Simulator,
    violations: Vec<crate::system::coherence_audit::Violation>,
) -> Vec<ViolationKind> {
    let line = line_of(sim, ADDR);
    violations
        .into_iter()
        .map(|violation| {
            assert_eq!(violation.line, line, "{violation:?} names another line");
            violation.kind
        })
        .collect()
}

fn coherence_faults(sim: &Simulator) -> Vec<ViolationKind> {
    kinds_at_addr(sim, sim.audit_coherence())
}

fn cache_faults(sim: &Simulator) -> Vec<ViolationKind> {
    kinds_at_addr(sim, sim.cache_violations())
}

fn cores(members: &[CoreId]) -> CoreSet {
    let mut set = CoreSet::EMPTY;
    for core in members {
        set.insert(*core);
    }
    set
}

#[test]
fn an_empty_system_breaks_no_invariant() {
    let sim = two_cores();

    assert_eq!(sim.cache_violations(), Vec::new());
}

#[test]
fn an_l1d_line_its_l2_lacks_breaks_coherence_and_inclusion() {
    let mut sim = two_cores();
    hold(l1d(&mut sim, CORE_0), MesiState::Shared);
    let (upper, lower) = (l1d(&mut sim, CORE_0).id, l2(&mut sim, CORE_0).id);

    assert_eq!(
        cache_faults(&sim),
        vec![
            ViolationKind::L1NotInL2 { core: CORE_0 },
            ViolationKind::Inclusion {
                upper,
                lower,
                fault: InclusionFault::MissingFromInclusiveLevel
            },
        ]
    );
}

#[test]
fn an_l1d_owning_a_line_its_l2_only_shares_is_reported() {
    let mut sim = two_cores();
    hold(l1d(&mut sim, CORE_0), MesiState::Modified);
    hold(l2(&mut sim, CORE_0), MesiState::Shared);
    grant(&mut sim, CORE_0, MesiState::Shared);

    assert_eq!(
        coherence_faults(&sim),
        vec![ViolationKind::L1DirtierThanL2 {
            core: CORE_0,
            l1: MesiState::Modified,
            l2: MesiState::Shared
        }]
    );
}

#[test]
fn a_copy_its_l2_does_not_record_is_reported() {
    let mut sim = two_cores();
    hold(l1d(&mut sim, CORE_0), MesiState::Shared);
    hold(l2(&mut sim, CORE_0), MesiState::Shared);
    grant(&mut sim, CORE_0, MesiState::Shared);
    let (upper, lower) = (l1d(&mut sim, CORE_0).id, l2(&mut sim, CORE_0).id);

    assert_eq!(
        cache_faults(&sim),
        vec![ViolationKind::Inclusion { upper, lower, fault: InclusionFault::UnrecordedCopy }]
    );
}

#[test]
fn a_line_held_above_and_in_an_exclusive_level_is_reported() {
    let mut sim = machine(1, InclusionPolicy::Exclusive);
    hold(l1d(&mut sim, CORE_0), MesiState::Shared);
    hold(l2(&mut sim, CORE_0), MesiState::Shared);
    let (upper, lower) = (l1d(&mut sim, CORE_0).id, l2(&mut sim, CORE_0).id);

    assert_eq!(
        cache_faults(&sim),
        vec![ViolationKind::Inclusion { upper, lower, fault: InclusionFault::InExclusiveLevel }]
    );
}

#[test]
fn two_cores_owning_a_line_are_reported() {
    let mut sim = two_cores();
    hold(l2(&mut sim, CORE_0), MesiState::Modified);
    hold(l2(&mut sim, CORE_1), MesiState::Exclusive);
    grant(&mut sim, CORE_0, MesiState::Modified);
    grant(&mut sim, CORE_1, MesiState::Shared);

    assert_eq!(coherence_faults(&sim), vec![ViolationKind::MultipleOwners(vec![CORE_0, CORE_1])]);
}

#[test]
fn an_owner_beside_a_sharer_is_reported() {
    let mut sim = two_cores();
    hold(l2(&mut sim, CORE_0), MesiState::Modified);
    hold(l2(&mut sim, CORE_1), MesiState::Shared);
    grant(&mut sim, CORE_0, MesiState::Modified);
    grant(&mut sim, CORE_1, MesiState::Shared);

    assert_eq!(
        coherence_faults(&sim),
        vec![ViolationKind::OwnerWithSharers { owner: CORE_0, others: 1 }]
    );
}

#[test]
fn a_line_the_home_does_not_track_is_reported() {
    let mut sim = two_cores();
    hold(l2(&mut sim, CORE_0), MesiState::Shared);

    assert_eq!(
        coherence_faults(&sim),
        vec![ViolationKind::HomeSharersMismatch {
            tracked: CoreSet::EMPTY,
            held: cores(&[CORE_0])
        }]
    );
}

#[test]
fn an_owner_the_home_records_as_a_sharer_is_reported() {
    let mut sim = two_cores();
    hold(l2(&mut sim, CORE_0), MesiState::Exclusive);
    grant(&mut sim, CORE_0, MesiState::Shared);

    assert_eq!(
        coherence_faults(&sim),
        vec![ViolationKind::HomeOwnerMismatch { tracked: None, held: Some(CORE_0) }]
    );
}

#[test]
fn a_line_the_home_tracks_but_no_l2_holds_is_reported() {
    let mut sim = two_cores();
    grant(&mut sim, CORE_0, MesiState::Shared);

    assert_eq!(coherence_faults(&sim), vec![ViolationKind::HomeTracksUnheldLine]);
}

/// An MSHR fetching `addr` for `targets` waiting requests.
fn mshr(cache: &Cache, seq: u64, addr: u64, targets: usize) -> Mshr {
    let target = MshrTarget {
        source: ComponentId::Cache(cache.id),
        req_id: ReqId::for_cache(cache.id, seq),
        paddr: PhysAddr::new(addr),
        vaddr: None,
        pc: None,
        size: AccessSize::B8,
        op: MemOp::Read,
    };
    Mshr {
        line: LineAddr::from_phys(PhysAddr::new(addr), cache.line_bytes() as u64),
        req_id: ReqId::for_cache(cache.id, seq),
        targets: vec![target; targets],
        deferred: Vec::new(),
        write: false,
        prefetch: false,
        issued_at: 0,
        upgrade: false,
    }
}

fn eviction(cache: &Cache, seq: u64, addr: u64) -> Writeback {
    Writeback {
        line: LineAddr::from_phys(PhysAddr::new(addr), cache.line_bytes() as u64),
        req_id: ReqId::for_cache(cache.id, seq),
        cause: WritebackCause::Eviction,
    }
}

/// `cache`'s bookkeeping faults, each checked to concern `ADDR`'s line.
fn bookkeeping_at_addr(cache: &Cache) -> Vec<BookkeepingFault> {
    let line = LineAddr::from_phys(PhysAddr::new(ADDR), cache.line_bytes() as u64);
    cache
        .bookkeeping_faults()
        .into_iter()
        .map(|(faulty, fault)| {
            assert_eq!(faulty, line, "{fault:?} names another line");
            fault
        })
        .collect()
}

#[test]
fn two_mshrs_fetching_one_line_are_reported() {
    let mut sim = machine(1, InclusionPolicy::default());
    let cache = l1d(&mut sim, CORE_0);
    for seq in 0..2 {
        let fetch = mshr(cache, seq, ADDR, 1);
        cache.plant_mshr(fetch);
    }

    assert_eq!(bookkeeping_at_addr(cache), vec![BookkeepingFault::FetchedByTwoMshrs]);
}

#[test]
fn an_mshr_over_its_target_limit_is_reported() {
    let mut config = Config::default();
    config.cache.l1_d.targets_per_mshr = std::num::NonZeroUsize::new(2).expect("2 is nonzero");
    let mut sim = Simulator::build(&config, "");
    let cache = l1d(&mut sim, CORE_0);
    let fetch = mshr(cache, 0, ADDR, 3);
    cache.plant_mshr(fetch);

    assert_eq!(
        bookkeeping_at_addr(cache),
        vec![BookkeepingFault::MshrOverTargets { targets: 3, limit: 2 }]
    );
}

#[test]
fn more_mshrs_than_the_cache_has_are_reported() {
    let mut config = Config::default();
    config.cache.l1_d.mshr_count = std::num::NonZeroUsize::new(2).expect("2 is nonzero");
    let mut sim = Simulator::build(&config, "");
    let cache = l1d(&mut sim, CORE_0);
    let line_bytes = cache.line_bytes() as u64;
    for seq in 0..3 {
        let fetch = mshr(cache, seq, ADDR + seq * line_bytes, 1);
        cache.plant_mshr(fetch);
    }

    let faults = cache.bookkeeping_faults();

    assert_eq!(
        faults,
        vec![(line_of(&sim, ADDR), BookkeepingFault::MshrsOverCapacity { held: 3, room: 2 })]
    );
}

#[test]
fn a_line_written_back_twice_at_once_is_reported() {
    let mut sim = machine(1, InclusionPolicy::default());
    let cache = l1d(&mut sim, CORE_0);
    for seq in 0..2 {
        let writeback = eviction(cache, seq, ADDR);
        cache.plant_writeback(writeback);
    }

    assert_eq!(bookkeeping_at_addr(cache), vec![BookkeepingFault::WrittenBackTwice]);
}

#[test]
fn more_writebacks_than_the_buffer_has_are_reported() {
    let mut config = Config::default();
    config.cache.l1_d.write_buffers = std::num::NonZeroUsize::new(2).expect("2 is nonzero");
    let mut sim = Simulator::build(&config, "");
    let cache = l1d(&mut sim, CORE_0);
    let line_bytes = cache.line_bytes() as u64;
    for seq in 0..3 {
        let writeback = eviction(cache, seq, ADDR + seq * line_bytes);
        cache.plant_writeback(writeback);
    }

    let faults = cache.bookkeeping_faults();

    assert_eq!(
        faults,
        vec![(line_of(&sim, ADDR), BookkeepingFault::WritebacksOverCapacity { taken: 3, room: 2 })]
    );
}
