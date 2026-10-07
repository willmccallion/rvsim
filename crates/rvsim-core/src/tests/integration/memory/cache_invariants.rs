//! Every cache invariant holds after every event: a program that sweeps,
//! strides, stores runs and cleans and flushes lines, on both backends,
//! under each inclusion policy, with and without every prefetcher.

use crate::config::{
    BackendKind, CacheConfig, Config, InclusionPolicy, LoadPrefetcherConfig, PageBoundary,
    PrefetcherKind, StorePrefetcherConfig,
};
use crate::isa::encoding::rv64i::{funct3 as i_f3, opcodes as i_op};
use crate::isa::encoding::zicboz::{CBO_CLEAN_IMM, CBO_FLUSH_IMM};
use crate::system::simulator::{StopAt, StopReason};
use crate::tests::integration::stats_accounting::program::{
    A1, BACKENDS, T0, T1, T2, config, ending_in_spin, system_with_configured_latency,
};
use crate::tests::support::builder::instruction::{FENCE_IORW, InstructionBuilder};

const POLICIES: [InclusionPolicy; 3] =
    [InclusionPolicy::Nine, InclusionPolicy::Inclusive, InclusionPolicy::Exclusive];

fn sized(cache: &mut CacheConfig, bytes: usize, ways: usize) {
    cache.enabled = true;
    cache.size_bytes = bytes;
    cache.ways = ways;
}

/// Small caches at every level, so lines are evicted, written back and
/// dropped at every level, under `policy`, with every prefetcher when
/// `prefetching`.
fn hierarchy(backend: BackendKind, policy: InclusionPolicy, prefetching: bool) -> Config {
    let mut config = config(backend);
    sized(&mut config.cache.l1_i, 2048, 2);
    sized(&mut config.cache.l1_d, 1024, 1);
    sized(&mut config.cache.l2, 4096, 4);
    sized(&mut config.cache.l3, 16 * 1024, 8);
    config.cache.inclusion_policy = policy;
    config.cache.wcb_entries = 4;
    if prefetching {
        for cache in [
            &mut config.cache.l1_i,
            &mut config.cache.l1_d,
            &mut config.cache.l2,
            &mut config.cache.l3,
        ] {
            cache.prefetcher = PrefetcherKind::NextLine;
        }
        config.cache.store_prefetcher = StorePrefetcherConfig::Stream { streams: 4, l2_lines: 2 };
        config.cache.load_prefetcher = LoadPrefetcherConfig::Stride {
            table_size: 64,
            l1_lines: 2,
            l2_lines: 4,
            page_boundary: PageBoundary::Stop,
        };
    }
    config
}

fn cbo(imm: i64, base: u32) -> u32 {
    ((imm as u32 & 0xFFF) << 20) | (base << 15) | (i_f3::CBO << 12) | i_op::OP_MISC_MEM
}

/// Reads and writes 256 lines, cleans one and flushes another, then
/// strides back over the first 64 with loads, and spins.
fn workout() -> (Vec<u32>, u64) {
    let i = InstructionBuilder::new;
    ending_in_spin(vec![
        i().addi(T1, 0, 256).build(),
        i().addi(T0, A1, 0).build(),
        i().ld(T2, T0, 0).build(),
        i().addi(T2, T2, 1).build(),
        i().sd(T0, T2, 0).build(),
        i().addi(T0, T0, 64).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -20).build(),
        cbo(CBO_CLEAN_IMM, A1),
        i().addi(T0, A1, 64).build(),
        cbo(CBO_FLUSH_IMM, T0),
        FENCE_IORW,
        i().addi(T1, 0, 64).build(),
        i().addi(T0, A1, 0).build(),
        i().ld(T2, T0, 0).build(),
        i().addi(T0, T0, 64).build(),
        i().addi(T1, T1, -1).build(),
        i().bne(T1, 0, -12).build(),
    ])
}

#[test]
fn every_cache_invariant_holds_after_every_event() {
    for backend in BACKENDS {
        for policy in POLICIES {
            for prefetching in [false, true] {
                let context = format!("{backend:?} {policy:?} prefetching {prefetching}");
                let config = hierarchy(backend, policy, prefetching);
                let (program, end) = workout();
                let mut ctx = system_with_configured_latency(&config, &program, &[]);
                ctx.sim.set_audit_caches(true);

                let stop = StopAt { pcs: vec![end], cycles: Some(500_000), ..StopAt::default() };
                let reason = ctx.sim.run_to(&stop);

                assert!(matches!(reason, Ok(StopReason::Pc { hart: 0 })), "{context}: {reason:?}");
            }
        }
    }
}
