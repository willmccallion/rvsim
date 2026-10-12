//! The checks that keep a configuration to machines the simulator can
//! build: every structure has room to work, sizes the hardware indexes as
//! powers of two are powers of two, nothing is too large to allocate, and
//! the address map has no overlaps.

use super::{
    BackendKind, CacheConfig, CacheHierarchyConfig, CoherenceConfig, Config, ConfigError,
    GeneralConfig, HomeAgentConfig, InterconnectConfig, LoadPrefetcherConfig, MemoryConfig,
    MemoryControllerKind, PipelineConfig, PrefetcherKind, ReplacementPolicyKind,
    StorePrefetcherConfig, SystemConfig,
};
use crate::isa::misa::Misa;

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
const LEAST_VECTOR_REGISTERS: usize = 32 + 8;
/// Physical integer and floating-point registers the architectural state
/// occupies.
const ARCHITECTURAL_REGISTERS: usize = 64;
/// The granule RAM is sized in.
const PAGE_BYTES: usize = 4096;

impl Config {
    /// Checks the machine's structures, sizes and address map.
    pub(super) fn check_machine(&self) -> Result<(), ConfigError> {
        check_system(&self.system, &self.general, self.memory.ram_size)?;
        check_memory(&self.memory)?;
        check_caches(&self.cache)?;
        check_pipeline(&self.pipeline, self.misa())?;
        check_predictors(&self.pipeline)?;
        check_coherence(&self.coherence)
    }
}

const fn at_least_one(field: &'static str, value: usize) -> Result<(), ConfigError> {
    if value == 0 { Err(ConfigError::Zero(field)) } else { Ok(()) }
}

const fn at_least_one_u64(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 { Err(ConfigError::Zero(field)) } else { Ok(()) }
}

const fn power_of_two(field: &'static str, value: usize) -> Result<(), ConfigError> {
    if value.is_power_of_two() { Ok(()) } else { Err(ConfigError::NotPowerOfTwo { field, value }) }
}

const fn at_most(field: &'static str, value: usize, most: usize) -> Result<(), ConfigError> {
    if value > most { Err(ConfigError::TooLarge { field, value, most }) } else { Ok(()) }
}

fn check_system(
    system: &SystemConfig,
    general: &GeneralConfig,
    ram_size: usize,
) -> Result<(), ConfigError> {
    at_least_one("system.hart_count", system.hart_count)?;
    at_least_one_u64("system.cpu_clock_mhz", system.cpu_clock_mhz)?;
    at_least_one_u64("system.clint_divider", system.clint_divider)?;
    at_least_one_u64("general.clock_hz", general.clock_hz)?;
    if !system.bus_width.is_power_of_two() {
        return Err(ConfigError::BusWidth(system.bus_width));
    }
    if ram_size == 0 || !ram_size.is_multiple_of(PAGE_BYTES) {
        return Err(ConfigError::RamSize(ram_size));
    }
    if system.kernel_offset >= ram_size as u64 {
        return Err(ConfigError::KernelOffset { offset: system.kernel_offset, ram_size });
    }
    check_address_map(system, ram_size)?;
    let names = system.device_regions().map(|region| region.name);
    if let Some(unknown) = system
        .device_latency_ns_overrides
        .keys()
        .find(|name| !names.contains(&name.as_str()) && name.as_str() != "HTIF")
    {
        return Err(ConfigError::UnknownDevice(unknown.clone()));
    }
    Ok(())
}

/// RAM and every device's register window must not share an address.
fn check_address_map(system: &SystemConfig, ram_size: usize) -> Result<(), ConfigError> {
    let ram = super::DeviceRegion { name: "RAM", base: system.ram_base, size: ram_size as u64 };
    let regions = system.device_regions();
    let all: Vec<_> = std::iter::once(ram).chain(regions).collect();
    for (index, first) in all.iter().enumerate() {
        if let Some(second) = all[index + 1..].iter().find(|other| first.overlaps(**other)) {
            return Err(ConfigError::RegionsOverlap { first: first.name, second: second.name });
        }
    }
    Ok(())
}

fn check_memory(memory: &MemoryConfig) -> Result<(), ConfigError> {
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
        if !config.enabled {
            continue;
        }
        check_cache_level(level, config)?;
        match first_enabled {
            Some((first, first_line_bytes)) if first_line_bytes != config.line_bytes => {
                return Err(ConfigError::LineSizesDiffer {
                    level,
                    line_bytes: config.line_bytes,
                    first,
                    first_line_bytes,
                });
            }
            Some(_) => {}
            None => first_enabled = Some((level, config.line_bytes)),
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
fn check_cache_level(level: &'static str, cache: &CacheConfig) -> Result<(), ConfigError> {
    if !cache.line_bytes.is_power_of_two() {
        return Err(ConfigError::NotPowerOfTwo {
            field: "cache line_bytes",
            value: cache.line_bytes,
        });
    }
    let set_bytes = cache.line_bytes.saturating_mul(cache.ways);
    let whole_sets = cache.ways > 0
        && cache.size_bytes >= set_bytes
        && cache.size_bytes.is_multiple_of(set_bytes);
    if !whole_sets {
        return Err(ConfigError::CacheGeometry {
            level,
            size_bytes: cache.size_bytes,
            ways: cache.ways,
            line_bytes: cache.line_bytes,
        });
    }
    if cache.policy == ReplacementPolicyKind::Plru && cache.ways > MAX_PLRU_WAYS {
        return Err(ConfigError::PlruWays { level, ways: cache.ways });
    }
    match cache.prefetcher {
        PrefetcherKind::Off => Ok(()),
        PrefetcherKind::Stride => {
            power_of_two("cache prefetch_table_size", cache.prefetch_table_size)?;
            at_least_one("cache prefetch_degree", cache.prefetch_degree)
        }
        PrefetcherKind::NextLine | PrefetcherKind::Stream | PrefetcherKind::Tagged => {
            at_least_one("cache prefetch_degree", cache.prefetch_degree)
        }
    }
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
            if size < ARCHITECTURAL_REGISTERS / 2 {
                return Err(ConfigError::ArchitecturalRegisters { field, size });
            }
        }
        if pipeline.prf_gpr_size + pipeline.prf_fpr_size <= ARCHITECTURAL_REGISTERS {
            return Err(ConfigError::NoFreeRegisters {
                gpr: pipeline.prf_gpr_size,
                fpr: pipeline.prf_fpr_size,
            });
        }
        if pipeline.prf_vpr_size < LEAST_VECTOR_REGISTERS {
            return Err(ConfigError::VectorRegisters(pipeline.prf_vpr_size));
        }
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
    let tage = &pipeline.tage;
    check_bank_list("tage", "history_lengths", tage.history_lengths.len(), tage.num_banks)?;
    check_bank_list("tage", "tag_widths", tage.tag_widths.len(), tage.num_banks)?;
    let ittage = &pipeline.ittage;
    at_most("ittage num_banks", ittage.num_banks, super::MAX_TAGE_BANKS)?;
    power_of_two("ittage table_size", ittage.table_size)?;
    check_bank_list("ittage", "history_lengths", ittage.history_lengths.len(), ittage.num_banks)?;
    check_bank_list("ittage", "tag_widths", ittage.tag_widths.len(), ittage.num_banks)?;
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
    at_most("loop_predictor log_size", pipeline.loop_predictor.log_size, MAX_LOOP_TABLE_BITS)?;
    at_least_one("store_set ssit_size", pipeline.store_set.ssit_size)?;
    at_least_one("store_set lfst_size", pipeline.store_set.lfst_size)
}

const fn check_bank_list(
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
