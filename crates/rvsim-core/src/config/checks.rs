//! The checks that keep a configuration to machines the simulator can
//! build: every structure has room to work, sizes the hardware indexes as
//! powers of two are powers of two, nothing is too large to allocate, and
//! the address map has no overlaps.

use super::pipeline::MAX_VECTOR_MEM_WIDTH;
use super::{
    BackendKind, CacheConfig, CacheHierarchyConfig, CoherenceConfig, Config, ConfigError,
    CsrSquash, HomeAgentConfig, InclusionPolicy, InterconnectConfig, LoadPrefetcherConfig,
    MemoryConfig, MemoryControllerKind, PipelineConfig, PrefetcherKind, ReplacementPolicyKind,
    StorePrefetcherConfig, SystemConfig, Timer,
};
use crate::common::PhysRange;
use crate::isa::encoding::zicboz::CBOZ_BLOCK_SIZE;
use crate::isa::misa::Misa;
use crate::isa::reg::RegIdx;
use crate::isa::rvv::{MAX_GROUP_REGS, VRegIdx};

/// The most ways PLRU replacement tracks: one bit per way in a 64-bit word,
/// with the all-ways-used state one past the last.
const MAX_PLRU_WAYS: usize = 63;
/// The most bits a predictor's global history mask holds.
const MAX_HISTORY_MASK_BITS: usize = 63;
/// The largest perceptron table, as a power of two.
const MAX_PERCEPTRON_TABLE_BITS: usize = 20;
/// The largest tournament predictor table, as a power of two.
const MAX_TOURNAMENT_TABLE_BITS: usize = 24;
/// The largest loop predictor, as a power of two.
const MAX_LOOP_TABLE_BITS: usize = 20;
/// The most write-combining buffer entries.
const MAX_WCB_ENTRIES: usize = 1024;
/// Vector registers rename needs: the 32 architectural ones and one
/// eight-register group for an LMUL=8 destination.
const LEAST_VECTOR_REGISTERS: usize = VRegIdx::COUNT + MAX_GROUP_REGS;
/// Physical registers a 16-bit `PhysReg` or `VecPhysReg` can number.
const MAX_PHYSICAL_REGISTERS: usize = 1 << u16::BITS;
/// Checkpoints an 8-bit `CheckpointId` can number.
const MAX_CHECKPOINTS: usize = 1 << u8::BITS;
/// Store sets a 16-bit `StoreSetId` can number.
const MAX_STORE_SETS: usize = 1 << u16::BITS;
/// SSIT entries a 32-bit `SsitIndex` can take a PC modulo.
const MAX_SSIT_ENTRIES: usize = u32::MAX as usize;
/// Bits of a loop-predictor tag or iteration count, held in 16-bit fields.
const MAX_LOOP_FIELD_BITS: usize = 16;
/// Bits of a loop-predictor confidence or age counter, held in 8-bit fields.
const MAX_LOOP_COUNTER_BITS: usize = 8;
/// The granule RAM is sized in.
const PAGE_BYTES: u64 = 4096;
/// The most harts: coherence tracks sharers in a 64-bit `CoreSet`.
const MAX_HARTS: usize = 64;

impl Config {
    /// The ISA string agrees with the vector unit's VLEN, and a V it names
    /// is the full extension.
    pub(super) fn check_isa(&self) -> Result<(), ConfigError> {
        if let Some(parsed) = self.parsed_isa().map_err(ConfigError::IsaString)?
            && let Some(named) = parsed.vlen
            && named != self.pipeline.vlen.bits()
        {
            return Err(ConfigError::VlenMismatch { named, configured: self.pipeline.vlen.bits() });
        }
        if self.misa().has_v() && !self.implements_full_v() {
            return Err(ConfigError::VWithoutFullVector {
                vlen: self.pipeline.vlen.bits(),
                elen: self.isa.vector.elen,
            });
        }
        Ok(())
    }

    /// No more harts than a sharer set holds, and several only with an
    /// inclusion policy coherence can keep.
    pub(super) fn check_harts(&self) -> Result<(), ConfigError> {
        let harts = self.system.hart_count.get();
        if harts > MAX_HARTS {
            return Err(ConfigError::TooManyHarts(harts));
        }
        if harts > 1 && self.cache.inclusion_policy == InclusionPolicy::Exclusive {
            return Err(ConfigError::ExclusiveWithCoherence);
        }
        Ok(())
    }

    /// Checks the machine's structures, sizes and address map.
    pub(super) fn check_machine(&self) -> Result<(), ConfigError> {
        check_system(&self.system, self.memory.ram_size)?;
        check_memory(&self.memory)?;
        check_caches(&self.cache)?;
        check_pipeline(&self.pipeline, self.misa())?;
        check_backend(&self.pipeline)?;
        check_predictors(&self.pipeline)?;
        check_coherence(&self.coherence)
    }
}

/// Stage latencies the backend can keep, a BTB of whole sets, a vector
/// memory port it can split lines into, and a squash policy the in-order
/// backend has.
fn check_backend(pipeline: &PipelineConfig) -> Result<(), ConfigError> {
    let ways = pipeline.btb_ways.max(1);
    let sets = (pipeline.btb_size / ways).max(1);
    if !sets.is_power_of_two() {
        return Err(ConfigError::BtbSets { size: pipeline.btb_size, ways, sets });
    }
    if pipeline.fetch_decode_latency == 0 {
        return Err(ConfigError::StageLatency { field: "fetch_decode_latency", least: 1 });
    }
    let least_rename_issue = match pipeline.backend {
        BackendKind::InOrder => 1,
        BackendKind::OutOfOrder => 2,
    };
    if pipeline.rename_issue_latency < least_rename_issue {
        return Err(ConfigError::StageLatency {
            field: "rename_issue_latency",
            least: least_rename_issue,
        });
    }
    if pipeline.backend == BackendKind::InOrder && pipeline.csr_squash == Some(CsrSquash::Never) {
        return Err(ConfigError::InOrderNeverSquashes);
    }
    let vector_mem_width = pipeline.vector_mem_width_bytes();
    if !vector_mem_width.is_power_of_two()
        || !(8..=MAX_VECTOR_MEM_WIDTH).contains(&vector_mem_width)
    {
        return Err(ConfigError::VectorMemWidth(vector_mem_width));
    }
    Ok(())
}

/// Refuses a zero `value` for `field`.
fn at_least_one<T: Copy + Default + PartialEq>(
    field: &'static str,
    value: T,
) -> Result<(), ConfigError> {
    if value == T::default() { Err(ConfigError::Zero(field)) } else { Ok(()) }
}

const fn power_of_two(field: &'static str, value: usize) -> Result<(), ConfigError> {
    if value.is_power_of_two() { Ok(()) } else { Err(ConfigError::NotPowerOfTwo { field, value }) }
}

const fn at_most(field: &'static str, value: usize, most: usize) -> Result<(), ConfigError> {
    if value > most { Err(ConfigError::TooLarge { field, value, most }) } else { Ok(()) }
}

fn check_system(system: &SystemConfig, ram_size: u64) -> Result<(), ConfigError> {
    if system.timer == Timer::RealTime
        && !system.cpu_clock_hz().get().is_multiple_of(u64::from(system.timebase_hz.get()))
    {
        return Err(ConfigError::TimebaseNotDivisor {
            timebase_hz: system.timebase_hz.get(),
            cpu_clock_hz: system.cpu_clock_hz().get(),
        });
    }
    if !system.bus_width.is_power_of_two() {
        return Err(ConfigError::BusWidth(system.bus_width));
    }
    if ram_size == 0 || !ram_size.is_multiple_of(PAGE_BYTES) {
        return Err(ConfigError::RamSize(ram_size));
    }
    if system.kernel_offset >= ram_size {
        return Err(ConfigError::KernelOffset { offset: system.kernel_offset, ram_size });
    }
    check_address_map(system, ram_size)?;
    Ok(())
}

/// RAM and every device's register window must not share an address.
fn check_address_map(system: &SystemConfig, ram_size: u64) -> Result<(), ConfigError> {
    let ram = super::DeviceRegion { name: "RAM", range: PhysRange::new(system.ram_base, ram_size) };
    let regions = system.device_regions();
    let all: Vec<_> = std::iter::once(ram).chain(regions).collect();
    for (index, first) in all.iter().enumerate() {
        if let Some(second) = all[index + 1..]
            .iter()
            .find(|other| first.range.overlaps(other.range.base, other.range.size))
        {
            return Err(ConfigError::RegionsOverlap { first: first.name, second: second.name });
        }
    }
    Ok(())
}

fn check_memory(memory: &MemoryConfig) -> Result<(), ConfigError> {
    if memory.simple_bandwidth_bytes_per_second().is_none() {
        return Err(ConfigError::SimpleBandwidth);
    }
    at_least_one("memory.tlb_size", memory.tlb_size)?;
    check_tlb("memory.tlb", memory.tlb_size, memory.tlb_ways)?;
    if memory.l2_tlb_size > 0 {
        check_tlb("memory.l2_tlb", memory.l2_tlb_size, memory.l2_tlb_ways)?;
    }
    if memory.controller == MemoryControllerKind::Dram {
        at_least_one("memory.num_banks", memory.num_banks)?;
        power_of_two("memory.row_size_bytes", memory.row_size_bytes)?;
    }
    Ok(())
}

/// A TLB with `ways` ways, 0 for fully associative, must fill a
/// power-of-two number of sets exactly.
const fn check_tlb(tlb: &'static str, entries: usize, ways: usize) -> Result<(), ConfigError> {
    if ways == 0 || ways == entries {
        return Ok(());
    }
    let fills_its_sets =
        ways < entries && entries.is_multiple_of(ways) && (entries / ways).is_power_of_two();
    if fills_its_sets { Ok(()) } else { Err(ConfigError::TlbGeometry { tlb, entries, ways }) }
}

fn check_caches(cache: &CacheHierarchyConfig) -> Result<(), ConfigError> {
    let levels =
        [("l1_i", &cache.l1_i), ("l1_d", &cache.l1_d), ("l2", &cache.l2), ("l3", &cache.l3)];
    let mut first_enabled: Option<(&'static str, usize)> = None;
    for (level, config) in levels {
        // The write log tracks lines of the L1D's size even when it is off,
        // and a disabled cache is still built from its geometry.
        check_geometry(level, config)?;
        if !config.enabled {
            continue;
        }
        check_cache_level(level, config)?;
        match first_enabled {
            Some((first, first_line_bytes)) if first_line_bytes != config.line_bytes.get() => {
                return Err(ConfigError::LineSizesDiffer {
                    level,
                    line_bytes: config.line_bytes.get(),
                    first,
                    first_line_bytes,
                });
            }
            Some(_) => {}
            None => first_enabled = Some((level, config.line_bytes.get())),
        }
    }
    at_most("cache.wcb_entries", cache.wcb_entries, MAX_WCB_ENTRIES)?;
    if let LoadPrefetcherConfig::Stride { table_size, .. } = cache.load_prefetcher {
        power_of_two("cache.load_prefetcher.table_size", table_size)?;
    }
    if let StorePrefetcherConfig::Stream { streams, .. } = cache.store_prefetcher {
        at_least_one("cache.store_prefetcher.streams", streams)?;
    }
    Ok(())
}

/// An enabled level holds a whole number of sets and its prefetcher has
/// something to do.
/// A level's lines are a power of two the caches and the write log can
/// index by, and its size is a whole number of sets.
const fn check_geometry(level: &'static str, cache: &CacheConfig) -> Result<(), ConfigError> {
    let (size_bytes, line_bytes, ways) =
        (cache.size_bytes.get(), cache.line_bytes.get(), cache.ways.get());
    if !line_bytes.is_power_of_two() {
        return Err(ConfigError::NotPowerOfTwo { field: "cache line_bytes", value: line_bytes });
    }
    let set_bytes = line_bytes.saturating_mul(ways);
    if size_bytes < set_bytes || !size_bytes.is_multiple_of(set_bytes) {
        return Err(ConfigError::CacheGeometry { level, size_bytes, ways, line_bytes });
    }
    Ok(())
}

/// An enabled level holds a cache block, a replacement policy that tracks
/// its ways, and a prefetcher table it can index.
fn check_cache_level(level: &'static str, cache: &CacheConfig) -> Result<(), ConfigError> {
    let line_bytes = cache.line_bytes.get();
    if (line_bytes as u64) < CBOZ_BLOCK_SIZE {
        return Err(ConfigError::LineSmallerThanCacheBlock { level, line_bytes });
    }
    if cache.policy == ReplacementPolicyKind::Plru && cache.ways.get() > MAX_PLRU_WAYS {
        return Err(ConfigError::PlruWays { level, ways: cache.ways.get() });
    }
    if cache.prefetcher == PrefetcherKind::Stride {
        power_of_two("cache prefetch_table_size", cache.prefetch_table_size)?;
    }
    Ok(())
}

fn check_pipeline(pipeline: &PipelineConfig, misa: Misa) -> Result<(), ConfigError> {
    at_least_one("pipeline.width", pipeline.width)?;
    for (field, width) in [
        ("pipeline.fetch_width", pipeline.fetch_width),
        ("pipeline.decode_width", pipeline.decode_width),
        ("pipeline.rename_width", pipeline.rename_width),
        ("pipeline.issue_width", pipeline.issue_width),
        ("pipeline.commit_width", pipeline.commit_width),
        ("pipeline.writeback_width", pipeline.writeback_width),
    ] {
        if let Some(width) = width {
            at_least_one(field, width)?;
        }
    }
    for (field, size) in [
        ("pipeline.rob_size", pipeline.rob_size),
        ("pipeline.issue_queue_size", pipeline.issue_queue_size),
        ("pipeline.store_buffer_size", pipeline.store_buffer_size),
        ("pipeline.load_queue_size", pipeline.load_queue_size),
        ("pipeline.load_ports", pipeline.load_ports),
        ("pipeline.store_ports", pipeline.store_ports),
        ("pipeline.squash_width", pipeline.squash_width),
        ("pipeline.vec_store_buffer_size", pipeline.vec_store_buffer_size),
    ] {
        at_least_one(field, size)?;
    }
    if let Some(lanes) = pipeline.num_vec_lanes {
        at_least_one("pipeline.num_vec_lanes", lanes)?;
    }
    if pipeline.backend == BackendKind::OutOfOrder {
        for (field, size) in [
            ("pipeline.prf_gpr_size", pipeline.prf_gpr_size),
            ("pipeline.prf_fpr_size", pipeline.prf_fpr_size),
        ] {
            if size < RegIdx::COUNT {
                return Err(ConfigError::ArchitecturalRegisters { field, size });
            }
        }
        if pipeline.prf_gpr_size + pipeline.prf_fpr_size <= 2 * RegIdx::COUNT {
            return Err(ConfigError::NoFreeRegisters {
                gpr: pipeline.prf_gpr_size,
                fpr: pipeline.prf_fpr_size,
            });
        }
        if pipeline.prf_vpr_size < LEAST_VECTOR_REGISTERS {
            return Err(ConfigError::VectorRegisters(pipeline.prf_vpr_size));
        }
        at_most(
            "pipeline.prf_gpr_size + pipeline.prf_fpr_size",
            pipeline.prf_gpr_size + pipeline.prf_fpr_size,
            MAX_PHYSICAL_REGISTERS,
        )?;
        at_most("pipeline.prf_vpr_size", pipeline.prf_vpr_size, MAX_PHYSICAL_REGISTERS)?;
        at_most("pipeline.checkpoint_count", pipeline.checkpoint_count, MAX_CHECKPOINTS)?;
    }
    check_units(pipeline, misa)
}

/// Every scalar instruction class the hart implements has a unit to run
/// on. A vector unit type left out gets one unit when the pool is built.
fn check_units(pipeline: &PipelineConfig, misa: Misa) -> Result<(), ConfigError> {
    let fu = &pipeline.fu_config;
    let mut needed = vec![
        ("IntAlu", fu.num_int_alu, "every program"),
        ("Branch", fu.num_branch, "every program"),
        ("Mem", fu.num_mem, "every program"),
    ];
    if misa.has('m') {
        needed.extend([("IntMul", fu.num_int_mul, "M"), ("IntDiv", fu.num_int_div, "M")]);
    }
    if misa.has('f') || misa.has('d') {
        needed.extend([
            ("FpAdd", fu.num_fp_add, "F and D"),
            ("FpMul", fu.num_fp_mul, "F and D"),
            ("FpFma", fu.num_fp_fma, "F and D"),
            ("FpDivSqrt", fu.num_fp_div_sqrt, "F and D"),
        ]);
    }
    match needed.into_iter().find(|&(_, count, _)| count == 0) {
        Some((unit, _, needed_by)) => Err(ConfigError::MissingUnit { unit, needed_by }),
        None => Ok(()),
    }
}

fn check_predictors(pipeline: &PipelineConfig) -> Result<(), ConfigError> {
    pipeline.tage.validate()?;
    pipeline.sc.validate()?;
    pipeline.bht.validate()?;
    let ittage = &pipeline.ittage;
    at_most("ittage num_banks", ittage.num_banks, super::MAX_TAGE_BANKS)?;
    power_of_two("ittage table_size", ittage.table_size)?;
    check_bank_list("ittage", "history_lengths", ittage.history_lengths.len(), ittage.num_banks)?;
    check_bank_list("ittage", "tag_widths", ittage.tag_widths.len(), ittage.num_banks)?;
    let longest_ittage_history = ittage.history_lengths.iter().copied().max().unwrap_or(0);
    at_most("ittage history_lengths", longest_ittage_history, super::MAX_ITTAGE_HISTORY)?;
    let perceptron = &pipeline.perceptron;
    at_most("perceptron history_length", perceptron.history_length, MAX_HISTORY_MASK_BITS)?;
    at_most("perceptron table_bits", perceptron.table_bits, MAX_PERCEPTRON_TABLE_BITS)?;
    let tournament = &pipeline.tournament;
    for (field, bits) in [
        ("tournament global_size_bits", tournament.global_size_bits),
        ("tournament local_hist_bits", tournament.local_hist_bits),
        ("tournament local_pred_bits", tournament.local_pred_bits),
    ] {
        at_most(field, bits, MAX_TOURNAMENT_TABLE_BITS)?;
    }
    let loop_predictor = &pipeline.loop_predictor;
    at_most("loop_predictor log_size", loop_predictor.log_size, MAX_LOOP_TABLE_BITS)?;
    at_most("loop_predictor tag_bits", loop_predictor.tag_bits, MAX_LOOP_FIELD_BITS)?;
    at_most("loop_predictor iter_bits", loop_predictor.iter_bits, MAX_LOOP_FIELD_BITS)?;
    at_most(
        "loop_predictor confidence_bits",
        loop_predictor.confidence_bits,
        MAX_LOOP_COUNTER_BITS,
    )?;
    at_most("loop_predictor age_bits", loop_predictor.age_bits, MAX_LOOP_COUNTER_BITS)?;
    let store_set = &pipeline.store_set;
    at_least_one("store_set ssit_size", store_set.ssit_size)?;
    at_most("store_set ssit_size", store_set.ssit_size, MAX_SSIT_ENTRIES)?;
    at_least_one("store_set lfst_size", store_set.lfst_size)?;
    at_most("store_set lfst_size", store_set.lfst_size, MAX_STORE_SETS)
}

pub(in crate::config) const fn check_bank_list(
    predictor: &'static str,
    field: &'static str,
    len: usize,
    banks: usize,
) -> Result<(), ConfigError> {
    if len == banks { Ok(()) } else { Err(ConfigError::BankList { predictor, field, len, banks }) }
}

fn check_coherence(coherence: &CoherenceConfig) -> Result<(), ConfigError> {
    at_least_one("coherence.txn_entries", coherence.txn_entries)?;
    if let HomeAgentConfig::SnoopFilter { capacity_factor, ways } = coherence.home_agent {
        if !(capacity_factor.is_finite() && capacity_factor > 0.0) {
            return Err(ConfigError::SnoopFilterCapacity);
        }
        at_least_one("coherence.home_agent.ways", ways)?;
    }
    let bytes_per_cycle = match coherence.interconnect {
        InterconnectConfig::Crossbar { bytes_per_cycle, .. }
        | InterconnectConfig::Ring { bytes_per_cycle, .. }
        | InterconnectConfig::Mesh { bytes_per_cycle, .. }
        | InterconnectConfig::Torus { bytes_per_cycle, .. }
        | InterconnectConfig::Hypercube { bytes_per_cycle, .. } => bytes_per_cycle,
    };
    at_least_one("coherence.interconnect.bytes_per_cycle", bytes_per_cycle)
}
