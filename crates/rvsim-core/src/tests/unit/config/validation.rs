//! Tests for the checks `Config::validate` applies before a system is built.

use crate::config::{
    BackendKind, BhtConfigError, CacheConfig, Config, ConfigError, CsrSquash, HomeAgentConfig,
    InterconnectConfig, LoadPrefetcherConfig, MAX_ITTAGE_HISTORY, MemoryControllerKind,
    PageBoundary, PrefetcherKind, ReplacementPolicyKind, StorePrefetcherConfig,
};

#[test]
fn default_config_validates() {
    assert!(Config::minimal().validate().is_ok());
}

#[test]
fn btb_with_a_power_of_two_set_count_validates() {
    let mut config = Config::minimal();
    config.pipeline.btb_size = 16384;
    config.pipeline.btb_ways = 8;

    assert!(config.validate().is_ok());
}

#[test]
fn btb_whose_set_count_is_not_a_power_of_two_is_rejected() {
    let mut config = Config::minimal();
    config.pipeline.btb_size = 12288;
    config.pipeline.btb_ways = 8;

    let err = config.validate().unwrap_err();

    assert!(matches!(err, ConfigError::BtbSets { size: 12288, ways: 8, sets: 1536 }), "{err}");
}

#[test]
fn a_simple_controller_bandwidth_that_is_not_positive_is_rejected() {
    let mut config = Config::minimal();
    config.memory.simple_bandwidth_gib_s = 0.0;

    let err = config.validate().unwrap_err();

    assert!(matches!(err, ConfigError::SimpleBandwidth), "{err}");
}

#[test]
fn a_cache_with_zero_mshrs_write_buffers_or_targets_is_refused() {
    for field in ["mshr_count", "write_buffers", "targets_per_mshr"] {
        let parsed = serde_json::from_str::<CacheConfig>(&format!(r#"{{"{field}": 0}}"#));

        let error = parsed.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(error.contains("nonzero"), "{field} = 0: {error:?}");
    }
}

#[test]
fn an_omitted_cache_resource_count_takes_its_default() {
    let cache: CacheConfig = serde_json::from_str(r#"{"size_bytes": 8192}"#).unwrap();

    assert_eq!(
        (cache.mshr_count.get(), cache.write_buffers.get(), cache.targets_per_mshr.get()),
        (8, 8, 20)
    );
}

#[test]
fn an_inorder_core_that_never_squashes_after_a_csr_is_rejected() {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::InOrder;
    config.pipeline.csr_squash = Some(CsrSquash::Never);

    let err = config.validate().unwrap_err();

    assert!(matches!(err, ConfigError::InOrderNeverSquashes), "{err}");
}

#[test]
fn an_out_of_order_core_may_never_squash_after_a_csr() {
    let mut config = Config::minimal();
    config.pipeline.backend = BackendKind::OutOfOrder;
    config.pipeline.csr_squash = Some(CsrSquash::Never);

    assert!(config.validate().is_ok());
}

#[test]
fn a_bht_whose_hashed_history_is_wider_than_its_index_is_rejected() {
    let mut config = Config::minimal();
    config.pipeline.bht.entries = 16;
    config.pipeline.bht.history_bits = 5;

    let err = config.validate().unwrap_err();

    assert!(
        matches!(
            err,
            ConfigError::Bht(BhtConfigError::HistoryBits { bits: 5, length: 8, index_bits: 4 })
        ),
        "{err}"
    );
}

#[test]
fn a_bht_with_three_bit_counters_is_rejected() {
    let mut config = Config::minimal();
    config.pipeline.bht.counter_bits = 3;

    let err = config.validate().unwrap_err();

    assert!(matches!(err, ConfigError::Bht(BhtConfigError::CounterBits(3))), "{err}");
}

/// The error `config` with `change` applied is refused with.
fn refused(mut config: Config, change: impl FnOnce(&mut Config)) -> ConfigError {
    change(&mut config);
    config.validate().expect_err("the config is refused")
}

/// The minimal machine with `change` applied, checked.
fn minimal_with(change: impl FnOnce(&mut Config)) -> Result<(), ConfigError> {
    let mut config = Config::minimal();
    change(&mut config);
    config.validate()
}

#[test]
fn the_default_machine_validates() {
    assert!(Config::default().validate().is_ok());
}

/// A structure with no room hangs or divides by zero, so zero is refused.
#[test]
fn a_structure_that_must_hold_something_refuses_zero() {
    type Change = fn(&mut Config);
    let cases: [(&str, Change); 26] = [
        ("pipeline.width", |c| c.pipeline.width = 0),
        ("pipeline.fetch_width", |c| c.pipeline.fetch_width = Some(0)),
        ("pipeline.issue_width", |c| c.pipeline.issue_width = Some(0)),
        ("pipeline.commit_width", |c| c.pipeline.commit_width = Some(0)),
        ("pipeline.rob_size", |c| c.pipeline.rob_size = 0),
        ("pipeline.issue_queue_size", |c| c.pipeline.issue_queue_size = 0),
        ("pipeline.store_buffer_size", |c| c.pipeline.store_buffer_size = 0),
        ("pipeline.load_queue_size", |c| c.pipeline.load_queue_size = 0),
        ("pipeline.load_ports", |c| c.pipeline.load_ports = 0),
        ("pipeline.store_ports", |c| c.pipeline.store_ports = 0),
        ("pipeline.squash_width", |c| c.pipeline.squash_width = 0),
        ("pipeline.vec_store_buffer_size", |c| c.pipeline.vec_store_buffer_size = 0),
        ("pipeline.num_vec_lanes", |c| c.pipeline.num_vec_lanes = Some(0)),
        ("memory.tlb_size", |c| c.memory.tlb_size = 0),
        ("system.hart_count", |c| c.system.hart_count = 0),
        ("system.cpu_clock_mhz", |c| c.system.cpu_clock_mhz = 0),
        ("system.clint_divider", |c| c.system.clint_divider = 0),
        ("general.clock_hz", |c| c.general.clock_hz = 0),
        ("coherence.txn_entries", |c| c.coherence.txn_entries = 0),
        ("store_set ssit_size", |c| c.pipeline.store_set.ssit_size = 0),
        ("store_set lfst_size", |c| c.pipeline.store_set.lfst_size = 0),
        ("cache.store_prefetcher.streams", |c| {
            c.cache.store_prefetcher = StorePrefetcherConfig::Stream { streams: 0, l2_lines: 8 };
        }),
        ("memory.num_banks", |c| {
            c.memory.controller = MemoryControllerKind::Dram;
            c.memory.num_banks = 0;
        }),
        ("coherence.home_agent.ways", |c| {
            c.coherence.home_agent = HomeAgentConfig::SnoopFilter { capacity_factor: 1.5, ways: 0 };
        }),
        ("coherence.interconnect.bytes_per_cycle", |c| {
            c.coherence.interconnect =
                InterconnectConfig::Mesh { hop_latency: 1, bytes_per_cycle: 0 };
        }),
        ("cache prefetch_degree", |c| {
            c.cache.l1_d.enabled = true;
            c.cache.l1_d.prefetcher = PrefetcherKind::NextLine;
            c.cache.l1_d.prefetch_degree = 0;
        }),
    ];
    for (field, change) in cases {
        let err = refused(Config::minimal(), change);

        assert_eq!(err, ConfigError::Zero(field), "{field}: {err}");
    }
}

#[test]
fn an_out_of_order_register_file_needs_registers_beyond_the_architectural_ones() {
    let err = refused(Config::default(), |c| {
        c.pipeline.prf_gpr_size = 32;
        c.pipeline.prf_fpr_size = 32;
    });

    assert_eq!(err, ConfigError::NoFreeRegisters { gpr: 32, fpr: 32 }, "{err}");
}

#[test]
fn an_out_of_order_register_file_holds_its_architectural_registers() {
    let err = refused(Config::default(), |c| c.pipeline.prf_gpr_size = 16);

    assert_eq!(
        err,
        ConfigError::ArchitecturalRegisters { field: "pipeline.prf_gpr_size", size: 16 },
        "{err}"
    );
}

#[test]
fn a_vector_register_file_needs_room_for_an_eight_register_group() {
    let small = refused(Config::default(), |c| c.pipeline.prf_vpr_size = 39);
    let mut enough = Config::default();
    enough.pipeline.prf_vpr_size = 40;

    assert_eq!(small, ConfigError::VectorRegisters(39), "{small}");
    assert!(enough.validate().is_ok());
}

#[test]
fn an_in_order_core_has_no_physical_register_file_to_size() {
    assert!(minimal_with(|c| c.pipeline.prf_vpr_size = 0).is_ok());
}

#[test]
fn an_implemented_instruction_class_needs_a_unit() {
    let alu = refused(Config::minimal(), |c| c.pipeline.fu_config.num_int_alu = 0);
    let multiplier = refused(Config::minimal(), |c| c.pipeline.fu_config.num_int_mul = 0);
    let fp_adder = refused(Config::minimal(), |c| c.pipeline.fu_config.num_fp_add = 0);

    assert_eq!(alu, ConfigError::MissingUnit { unit: "IntAlu", needed_by: "every program" });
    assert_eq!(multiplier, ConfigError::MissingUnit { unit: "IntMul", needed_by: "M" });
    assert_eq!(fp_adder, ConfigError::MissingUnit { unit: "FpAdd", needed_by: "F and D" });
}

#[test]
fn a_hart_without_m_needs_no_multiplier() {
    let result = minimal_with(|c| {
        c.isa.string = Some("rv64iafdc".to_owned());
        c.pipeline.fu_config.num_int_mul = 0;
        c.pipeline.fu_config.num_int_div = 0;
    });

    assert!(result.is_ok(), "{result:?}");
}

/// The minimal machine with an enabled L1D of `size_bytes` in `ways` ways
/// of `line_bytes`-byte lines, checked.
fn with_l1_d(size_bytes: usize, ways: usize, line_bytes: usize) -> Result<(), ConfigError> {
    minimal_with(|c| {
        c.cache.l1_d =
            CacheConfig { enabled: true, size_bytes, ways, line_bytes, ..CacheConfig::default() };
    })
}

#[test]
fn an_enabled_cache_holds_a_whole_number_of_sets() {
    let geometry = |size_bytes, ways, line_bytes| ConfigError::CacheGeometry {
        level: "l1_d",
        size_bytes,
        ways,
        line_bytes,
    };

    assert_eq!(with_l1_d(32 << 10, 3, 64), Err(geometry(32 << 10, 3, 64)));
    assert_eq!(with_l1_d(32 << 10, 0, 64), Err(geometry(32 << 10, 0, 64)));
    assert_eq!(with_l1_d(0, 4, 64), Err(geometry(0, 4, 64)));
    assert_eq!(with_l1_d(32 << 10, 1024, 64), Err(geometry(32 << 10, 1024, 64)));
    assert!(with_l1_d(48 << 10, 12, 64).is_ok(), "64 sets of 12 ways");
    assert!(with_l1_d(48 << 10, 4, 64).is_ok(), "192 sets, indexed by modulo");
}

#[test]
fn a_cache_line_is_a_power_of_two() {
    assert_eq!(
        with_l1_d(96 * 64, 1, 96),
        Err(ConfigError::NotPowerOfTwo { field: "cache line_bytes", value: 96 })
    );
}

#[test]
fn plru_replacement_tracks_at_most_63_ways() {
    let plru = |ways: usize| {
        minimal_with(|c| {
            c.cache.l1_d = CacheConfig {
                enabled: true,
                size_bytes: ways * 64,
                ways,
                policy: ReplacementPolicyKind::Plru,
                ..CacheConfig::default()
            };
        })
    };

    assert_eq!(plru(64), Err(ConfigError::PlruWays { level: "l1_d", ways: 64 }));
    assert!(plru(32).is_ok());
}

#[test]
fn every_cache_level_shares_one_line_size() {
    let err = refused(Config::default(), |c| {
        c.cache.l2.line_bytes = 128;
    });

    assert_eq!(
        err,
        ConfigError::LineSizesDiffer {
            level: "l2",
            line_bytes: 128,
            first: "l1_i",
            first_line_bytes: 64
        },
        "{err}"
    );
}

#[test]
fn a_stride_prefetcher_table_is_a_power_of_two() {
    let cache_side = refused(Config::minimal(), |c| {
        c.cache.l1_d = CacheConfig {
            enabled: true,
            prefetcher: PrefetcherKind::Stride,
            prefetch_table_size: 100,
            ..CacheConfig::default()
        };
    });
    let load_side = refused(Config::minimal(), |c| {
        c.cache.load_prefetcher = LoadPrefetcherConfig::Stride {
            table_size: 100,
            l1_lines: 4,
            l2_lines: 0,
            page_boundary: PageBoundary::Stop,
        };
    });

    assert_eq!(
        cache_side,
        ConfigError::NotPowerOfTwo { field: "cache prefetch_table_size", value: 100 }
    );
    assert_eq!(
        load_side,
        ConfigError::NotPowerOfTwo { field: "cache.load_prefetcher.table_size", value: 100 }
    );
}

#[test]
fn a_set_associative_tlb_fills_a_power_of_two_number_of_sets() {
    let l1 = refused(Config::minimal(), |c| c.memory.tlb_ways = 3);
    let l2 = refused(Config::minimal(), |c| {
        c.memory.l2_tlb_size = 100;
        c.memory.l2_tlb_ways = 8;
    });

    assert_eq!(l1, ConfigError::TlbGeometry { tlb: "memory.tlb", entries: 64, ways: 3 });
    assert_eq!(l2, ConfigError::TlbGeometry { tlb: "memory.l2_tlb", entries: 100, ways: 8 });
    assert!(minimal_with(|c| c.memory.tlb_ways = 8).is_ok());
    assert!(minimal_with(|c| c.memory.tlb_ways = 0).is_ok(), "fully associative");
    assert!(
        minimal_with(|c| {
            c.memory.l2_tlb_size = 1024;
            c.memory.l2_tlb_ways = 0;
        })
        .is_ok()
    );
}

#[test]
fn ram_is_a_whole_number_of_pages_holding_the_kernel_offset() {
    let empty = refused(Config::minimal(), |c| c.memory.ram_size = 0);
    let partial = refused(Config::minimal(), |c| c.memory.ram_size = 1000);
    let past = refused(Config::minimal(), |c| c.system.kernel_offset = 1 << 40);

    assert_eq!(empty, ConfigError::RamSize(0));
    assert_eq!(partial, ConfigError::RamSize(1000));
    assert!(matches!(past, ConfigError::KernelOffset { offset: 0x100_0000_0000, .. }), "{past}");
}

#[test]
fn no_two_regions_of_the_address_map_overlap() {
    let uart_in_ram = refused(Config::minimal(), |c| c.system.uart_base = 0x8000_1000);
    let ram_over_disk = refused(Config::minimal(), |c| c.memory.ram_size = 512 << 20);

    assert_eq!(uart_in_ram, ConfigError::RegionsOverlap { first: "RAM", second: "UART0" });
    assert_eq!(ram_over_disk, ConfigError::RegionsOverlap { first: "RAM", second: "VirtIO-Blk" });
}

#[test]
fn a_latency_override_names_a_device_the_system_has() {
    let typo = refused(Config::minimal(), |c| {
        c.system.device_latency_ns_overrides.insert("UARTT0".to_owned(), 5);
    });
    let known = minimal_with(|c| {
        c.system.device_latency_ns_overrides.insert("UART0".to_owned(), 5);
        c.system.device_latency_ns_overrides.insert("HTIF".to_owned(), 5);
    });

    assert_eq!(typo, ConfigError::UnknownDevice("UARTT0".to_owned()));
    assert!(known.is_ok(), "{known:?}");
}

#[test]
fn the_bus_is_a_power_of_two_bytes_wide() {
    assert_eq!(refused(Config::minimal(), |c| c.system.bus_width = 3), ConfigError::BusWidth(3));
    assert_eq!(refused(Config::minimal(), |c| c.system.bus_width = 0), ConfigError::BusWidth(0));
}

#[test]
fn a_tage_table_size_is_a_power_of_two() {
    let err = refused(Config::minimal(), |c| c.pipeline.tage.table_size = 1000);

    assert_eq!(err, ConfigError::NotPowerOfTwo { field: "tage table_size", value: 1000 }, "{err}");
}

#[test]
fn a_tagged_predictors_lists_have_one_entry_per_bank() {
    let tage = refused(Config::minimal(), |c| c.pipeline.tage.history_lengths = vec![5, 11]);
    let ittage = refused(Config::minimal(), |c| c.pipeline.ittage.tag_widths = vec![9]);

    assert_eq!(
        tage,
        ConfigError::BankList { predictor: "tage", field: "history_lengths", len: 2, banks: 8 }
    );
    assert_eq!(
        ittage,
        ConfigError::BankList { predictor: "ittage", field: "tag_widths", len: 1, banks: 8 }
    );
}

#[test]
fn an_ittage_history_longer_than_the_global_history_is_refused() {
    let err = refused(Config::minimal(), |c| {
        c.pipeline.ittage.history_lengths =
            vec![4, 8, 16, 32, 64, 128, 256, MAX_ITTAGE_HISTORY + 1];
    });

    assert_eq!(
        err,
        ConfigError::TooLarge {
            field: "ittage history_lengths",
            value: MAX_ITTAGE_HISTORY + 1,
            most: MAX_ITTAGE_HISTORY,
        }
    );
}

#[test]
fn an_ittage_table_size_is_a_power_of_two() {
    let err = refused(Config::minimal(), |c| c.pipeline.ittage.table_size = 300);

    assert_eq!(err, ConfigError::NotPowerOfTwo { field: "ittage table_size", value: 300 });
}

#[test]
fn a_predictor_table_too_large_to_allocate_is_refused() {
    let perceptron = refused(Config::minimal(), |c| c.pipeline.perceptron.table_bits = 40);
    let history = refused(Config::minimal(), |c| c.pipeline.perceptron.history_length = 64);
    let tournament = refused(Config::minimal(), |c| c.pipeline.tournament.global_size_bits = 30);
    let loop_table = refused(Config::minimal(), |c| c.pipeline.loop_predictor.log_size = 30);
    let wcb = refused(Config::minimal(), |c| c.cache.wcb_entries = 1 << 30);

    assert_eq!(
        perceptron,
        ConfigError::TooLarge { field: "perceptron table_bits", value: 40, most: 20 }
    );
    assert_eq!(
        history,
        ConfigError::TooLarge { field: "perceptron history_length", value: 64, most: 63 }
    );
    assert_eq!(
        tournament,
        ConfigError::TooLarge { field: "tournament global_size_bits", value: 30, most: 24 }
    );
    assert_eq!(
        loop_table,
        ConfigError::TooLarge { field: "loop_predictor log_size", value: 30, most: 20 }
    );
    assert_eq!(
        wcb,
        ConfigError::TooLarge { field: "cache.wcb_entries", value: 1 << 30, most: 1024 }
    );
}

#[test]
fn a_dram_row_is_a_power_of_two_bytes() {
    let err = refused(Config::minimal(), |c| {
        c.memory.controller = MemoryControllerKind::Dram;
        c.memory.row_size_bytes = 1000;
    });

    assert_eq!(err, ConfigError::NotPowerOfTwo { field: "memory.row_size_bytes", value: 1000 });
}

#[test]
fn a_snoop_filter_has_a_positive_capacity() {
    for capacity_factor in [0.0, -1.0, f64::NAN] {
        let err = refused(Config::minimal(), |c| {
            c.coherence.home_agent = HomeAgentConfig::SnoopFilter { capacity_factor, ways: 8 };
        });

        assert_eq!(err, ConfigError::SnoopFilterCapacity, "{capacity_factor}");
    }
}

/// Settings that look odd but describe machines that work.
#[test]
fn degenerate_but_working_machines_validate() {
    type Change = fn(&mut Config);
    let cases: [(&str, Change); 6] = [
        ("no BTB", |c| c.pipeline.btb_size = 0),
        ("no return stack", |c| c.pipeline.ras_size = 0),
        ("a zero-latency ALU", |c| c.pipeline.fu_config.int_alu_latency = 0),
        ("zero-latency memory", |c| c.memory.simple_latency = 0),
        ("an L2 smaller than the L1D", |c| {
            c.cache.l1_d = CacheConfig {
                enabled: true,
                size_bytes: 32 << 10,
                ways: 4,
                ..CacheConfig::default()
            };
            c.cache.l2 = CacheConfig {
                enabled: true,
                size_bytes: 16 << 10,
                ways: 4,
                ..CacheConfig::default()
            };
        }),
        ("an in-order core's unused queues", |c| c.pipeline.backend = BackendKind::InOrder),
    ];
    for (name, change) in cases {
        let result = minimal_with(change);

        assert!(result.is_ok(), "{name}: {result:?}");
    }
}
