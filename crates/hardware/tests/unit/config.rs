//! # Configuration Tests
//!
//! Comprehensive tests for configuration structures, deserialization,
//! defaults, and validation.

use rvsim_core::config::*;

#[test]
fn test_config_default() {
    let config = Config::default();
    assert!(!config.general.trace_instructions);
    assert_eq!(config.general.start_pc, 0x8000_0000);
    assert!(config.general.direct_mode);
    assert_eq!(config.general.initial_sp, None);
}

#[test]
fn test_general_config_defaults() {
    let general = GeneralConfig::default();
    assert!(!general.trace_instructions);
    assert_eq!(general.start_pc, 0x8000_0000);
    assert!(general.direct_mode);
    assert_eq!(general.initial_sp, None);
}

#[test]
fn test_system_config_defaults() {
    let system = SystemConfig::default();
    assert_eq!(system.uart_base, 0x1000_0000);
    assert_eq!(system.disk_base, 0x9000_0000);
    assert_eq!(system.ram_base, 0x8000_0000);
    assert_eq!(system.clint_base, 0x0200_0000);
    assert_eq!(system.syscon_base, 0x0010_0000);
    assert_eq!(system.kernel_offset, 0x0020_0000);
    assert_eq!(system.bus_width, 8);
    assert_eq!(system.bus_latency, 4);
    assert_eq!(system.clint_divider, 10);
    assert!(!system.uart_to_stderr);
}

#[test]
fn test_memory_config_defaults() {
    let memory = MemoryConfig::default();
    assert_eq!(memory.ram_size, 128 * 1024 * 1024);
    assert_eq!(memory.controller, MemoryController::Simple);
    assert_eq!(memory.t_cas, 14);
    assert_eq!(memory.t_ras, 14);
    assert_eq!(memory.t_pre, 14);
    assert_eq!(memory.row_miss_latency, 120);
    assert_eq!(memory.tlb_size, 32);
}

#[test]
fn test_cache_config_defaults() {
    let cache = CacheConfig::default();
    assert!(!cache.enabled);
    assert_eq!(cache.size_bytes, 4096);
    assert_eq!(cache.line_bytes, 64);
    assert_eq!(cache.ways, 1);
    assert_eq!(cache.policy, ReplacementPolicy::Lru);
    assert_eq!(cache.latency, 1);
    assert_eq!(cache.prefetcher, Prefetcher::None);
    assert_eq!(cache.prefetch_table_size, 64);
    assert_eq!(cache.prefetch_degree, 1);
}

#[test]
fn test_cache_hierarchy_defaults() {
    let hierarchy = CacheHierarchyConfig::default();
    assert!(!hierarchy.l1_i.enabled);
    assert!(!hierarchy.l1_d.enabled);
    assert!(!hierarchy.l2.enabled);
    assert!(!hierarchy.l3.enabled);
}

#[test]
fn test_pipeline_config_defaults() {
    let pipeline = PipelineConfig::default();
    assert_eq!(pipeline.width, 1);
    assert_eq!(pipeline.branch_predictor, BranchPredictor::Static);
    assert_eq!(pipeline.btb_size, 256);
    assert_eq!(pipeline.ras_size, 8);
    assert_eq!(pipeline.misa_override, None);
}

#[test]
fn test_tage_config_defaults() {
    let tage = TageConfig::default();
    assert_eq!(tage.num_banks, 0);
    assert_eq!(tage.table_size, 0);
    assert_eq!(tage.loop_table_size, 0);
    assert_eq!(tage.reset_interval, 0);
    assert_eq!(tage.history_lengths, Vec::<usize>::new());
    assert_eq!(tage.tag_widths, Vec::<usize>::new());
}

#[test]
fn test_perceptron_config_defaults() {
    let perceptron = PerceptronConfig::default();
    assert_eq!(perceptron.history_length, 0);
    assert_eq!(perceptron.table_bits, 0);
}

#[test]
fn test_tournament_config_defaults() {
    let tournament = TournamentConfig::default();
    assert_eq!(tournament.global_size_bits, 0);
    assert_eq!(tournament.local_hist_bits, 0);
    assert_eq!(tournament.local_pred_bits, 0);
}

#[test]
fn test_memory_controller_enum() {
    assert_eq!(MemoryController::default(), MemoryController::Simple);
    assert_ne!(MemoryController::Simple, MemoryController::Dram);
}

#[test]
fn test_replacement_policy_enum() {
    assert_eq!(ReplacementPolicy::default(), ReplacementPolicy::Lru);
    assert_ne!(ReplacementPolicy::Lru, ReplacementPolicy::Fifo);
    assert_ne!(ReplacementPolicy::Lru, ReplacementPolicy::Random);
    assert_ne!(ReplacementPolicy::Lru, ReplacementPolicy::Mru);
    assert_ne!(ReplacementPolicy::Lru, ReplacementPolicy::Plru);
}

#[test]
fn test_prefetcher_enum() {
    assert_eq!(Prefetcher::default(), Prefetcher::None);
    assert_ne!(Prefetcher::None, Prefetcher::NextLine);
    assert_ne!(Prefetcher::None, Prefetcher::Stride);
    assert_ne!(Prefetcher::None, Prefetcher::Stream);
    assert_ne!(Prefetcher::None, Prefetcher::Tagged);
}

#[test]
fn test_branch_predictor_enum() {
    assert_eq!(BranchPredictor::default(), BranchPredictor::Static);
    assert_ne!(BranchPredictor::Static, BranchPredictor::GShare);
    assert_ne!(BranchPredictor::Static, BranchPredictor::Perceptron);
    assert_ne!(BranchPredictor::Static, BranchPredictor::Tage);
    assert_ne!(BranchPredictor::Static, BranchPredictor::Tournament);
}

#[test]
fn test_json_deserialization_minimal() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Simple",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert!(!config.general.trace_instructions);
    assert_eq!(config.general.start_pc, 0x8000_0000);
}

#[test]
fn test_json_deserialization_with_tracing() {
    let json = r#"{
        "general": {
            "trace_instructions": true,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Simple",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": true,
                "size_bytes": 32768,
                "line_bytes": 64,
                "ways": 4,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "NextLine",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": true,
                "size_bytes": 32768,
                "line_bytes": 64,
                "ways": 4,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "Stride",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": true,
                "size_bytes": 131072,
                "line_bytes": 64,
                "ways": 8,
                "policy": "LRU",
                "latency": 10,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 20,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "GShare",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.general.trace_instructions);
    assert!(config.cache.l1_i.enabled);
    assert!(config.cache.l1_d.enabled);
    assert!(config.cache.l2.enabled);
    assert!(!config.cache.l3.enabled);
    assert_eq!(config.cache.l1_d.size_bytes, 32768);
    assert_eq!(config.cache.l1_d.ways, 4);
    assert_eq!(config.cache.l1_d.prefetcher, Prefetcher::Stride);
    assert_eq!(config.pipeline.branch_predictor, BranchPredictor::GShare);
}

#[test]
fn test_json_dram_controller() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Dram",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.memory.controller, MemoryController::Dram);
}

#[test]
fn test_json_all_replacement_policies() {
    for policy in &["LRU", "FIFO", "RANDOM", "MRU", "PLRU"] {
        let json = format!(
            r#"{{
            "general": {{"trace_instructions": false, "start_pc": 2147483648, "direct_mode": true}},
            "system": {{"ram_base": 2147483648, "uart_base": 268435456, "disk_base": 2415919104, "clint_base": 33554432, "syscon_base": 1048576, "kernel_offset": 2097152, "bus_width": 8, "bus_latency": 4, "clint_divider": 10, "uart_to_stderr": false}},
            "memory": {{"ram_size": 134217728, "controller": "Simple", "t_cas": 14, "t_ras": 14, "t_pre": 14, "row_miss_latency": 120, "tlb_size": 32}},
            "cache": {{
                "l1_i": {{"enabled": true, "size_bytes": 4096, "line_bytes": 64, "ways": 4, "policy": "{}", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l1_d": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l2": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l3": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}}
            }},
            "pipeline": {{"width": 1, "branch_predictor": "Static", "btb_size": 256, "ras_size": 8, "tage": {{"num_banks": 4, "table_size": 2048, "loop_table_size": 256, "reset_interval": 256000, "history_lengths": [5, 15, 44, 130], "tag_widths": [9, 9, 10, 10]}}, "perceptron": {{"history_length": 32, "table_bits": 10}}, "tournament": {{"global_size_bits": 12, "local_hist_bits": 10, "local_pred_bits": 10}}}}
        }}"#,
            policy
        );
        let config: Config = serde_json::from_str(&json).unwrap();
        assert!(config.cache.l1_i.enabled);
    }
}

#[test]
fn test_json_all_prefetchers() {
    for prefetcher in &["None", "NextLine", "Stride", "Stream", "Tagged"] {
        let json = format!(
            r#"{{
            "general": {{"trace_instructions": false, "start_pc": 2147483648, "direct_mode": true}},
            "system": {{"ram_base": 2147483648, "uart_base": 268435456, "disk_base": 2415919104, "clint_base": 33554432, "syscon_base": 1048576, "kernel_offset": 2097152, "bus_width": 8, "bus_latency": 4, "clint_divider": 10, "uart_to_stderr": false}},
            "memory": {{"ram_size": 134217728, "controller": "Simple", "t_cas": 14, "t_ras": 14, "t_pre": 14, "row_miss_latency": 120, "tlb_size": 32}},
            "cache": {{
                "l1_i": {{"enabled": true, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "{}", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l1_d": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l2": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l3": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}}
            }},
            "pipeline": {{"width": 1, "branch_predictor": "Static", "btb_size": 256, "ras_size": 8, "tage": {{"num_banks": 4, "table_size": 2048, "loop_table_size": 256, "reset_interval": 256000, "history_lengths": [5, 15, 44, 130], "tag_widths": [9, 9, 10, 10]}}, "perceptron": {{"history_length": 32, "table_bits": 10}}, "tournament": {{"global_size_bits": 12, "local_hist_bits": 10, "local_pred_bits": 10}}}}
        }}"#,
            prefetcher
        );
        let config: Config = serde_json::from_str(&json).unwrap();
        assert!(config.cache.l1_i.enabled);
    }
}

#[test]
fn test_json_all_branch_predictors() {
    for predictor in &["Static", "GShare", "Perceptron", "Tage", "Tournament"] {
        let json = format!(
            r#"{{
            "general": {{"trace_instructions": false, "start_pc": 2147483648, "direct_mode": true}},
            "system": {{"ram_base": 2147483648, "uart_base": 268435456, "disk_base": 2415919104, "clint_base": 33554432, "syscon_base": 1048576, "kernel_offset": 2097152, "bus_width": 8, "bus_latency": 4, "clint_divider": 10, "uart_to_stderr": false}},
            "memory": {{"ram_size": 134217728, "controller": "Simple", "t_cas": 14, "t_ras": 14, "t_pre": 14, "row_miss_latency": 120, "tlb_size": 32}},
            "cache": {{
                "l1_i": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l1_d": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l2": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}},
                "l3": {{"enabled": false, "size_bytes": 4096, "line_bytes": 64, "ways": 1, "policy": "LRU", "latency": 1, "prefetcher": "None", "prefetch_table_size": 64, "prefetch_degree": 1}}
            }},
            "pipeline": {{"width": 1, "branch_predictor": "{}", "btb_size": 256, "ras_size": 8, "tage": {{"num_banks": 4, "table_size": 2048, "loop_table_size": 256, "reset_interval": 256000, "history_lengths": [5, 15, 44, 130], "tag_widths": [9, 9, 10, 10]}}, "perceptron": {{"history_length": 32, "table_bits": 10}}, "tournament": {{"global_size_bits": 12, "local_hist_bits": 10, "local_pred_bits": 10}}}}
        }}"#,
            predictor
        );
        let config: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config.general.start_pc, 0x8000_0000);
    }
}

#[test]
fn test_initial_sp_option() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true,
            "initial_sp": 2148532224
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Simple",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.general.initial_sp, Some(0x8010_0000));
}

#[test]
fn test_misa_override_option() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Simple",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "misa_override": "RV64IMAFDC",
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.pipeline.misa_override,
        Some("RV64IMAFDC".to_string())
    );
}

#[test]
fn test_uart_to_stderr_flag() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": true
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Simple",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.system.uart_to_stderr);
}

#[test]
fn test_custom_cache_sizes() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Simple",
            "t_cas": 14,
            "t_ras": 14,
            "t_pre": 14,
            "row_miss_latency": 120,
            "tlb_size": 32
        },
        "cache": {
            "l1_i": {
                "enabled": true,
                "size_bytes": 16384,
                "line_bytes": 64,
                "ways": 2,
                "policy": "LRU",
                "latency": 2,
                "prefetcher": "None",
                "prefetch_table_size": 128,
                "prefetch_degree": 2
            },
            "l1_d": {
                "enabled": true,
                "size_bytes": 16384,
                "line_bytes": 64,
                "ways": 2,
                "policy": "LRU",
                "latency": 2,
                "prefetcher": "None",
                "prefetch_table_size": 128,
                "prefetch_degree": 2
            },
            "l2": {
                "enabled": true,
                "size_bytes": 262144,
                "line_bytes": 64,
                "ways": 8,
                "policy": "LRU",
                "latency": 15,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": true,
                "size_bytes": 1048576,
                "line_bytes": 64,
                "ways": 16,
                "policy": "LRU",
                "latency": 40,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.cache.l1_i.size_bytes, 16384);
    assert_eq!(config.cache.l1_i.ways, 2);
    assert_eq!(config.cache.l1_i.latency, 2);
    assert_eq!(config.cache.l1_i.prefetch_table_size, 128);
    assert_eq!(config.cache.l1_i.prefetch_degree, 2);
    assert_eq!(config.cache.l2.size_bytes, 262144);
    assert_eq!(config.cache.l2.latency, 15);
    assert_eq!(config.cache.l3.size_bytes, 1048576);
    assert_eq!(config.cache.l3.ways, 16);
    assert_eq!(config.cache.l3.latency, 40);
    assert!(config.cache.l3.enabled);
}

#[test]
fn test_custom_dram_timings() {
    let json = r#"{
        "general": {
            "trace_instructions": false,
            "start_pc": 2147483648,
            "direct_mode": true
        },
        "system": {
            "ram_base": 2147483648,
            "uart_base": 268435456,
            "disk_base": 2415919104,
            "clint_base": 33554432,
            "syscon_base": 1048576,
            "kernel_offset": 2097152,
            "bus_width": 8,
            "bus_latency": 4,
            "clint_divider": 10,
            "uart_to_stderr": false
        },
        "memory": {
            "ram_size": 134217728,
            "controller": "Dram",
            "t_cas": 20,
            "t_ras": 45,
            "t_pre": 20,
            "row_miss_latency": 200,
            "tlb_size": 64
        },
        "cache": {
            "l1_i": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l1_d": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l2": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            },
            "l3": {
                "enabled": false,
                "size_bytes": 4096,
                "line_bytes": 64,
                "ways": 1,
                "policy": "LRU",
                "latency": 1,
                "prefetcher": "None",
                "prefetch_table_size": 64,
                "prefetch_degree": 1
            }
        },
        "pipeline": {
            "width": 1,
            "branch_predictor": "Static",
            "btb_size": 256,
            "ras_size": 8,
            "tage": {
                "num_banks": 4,
                "table_size": 2048,
                "loop_table_size": 256,
                "reset_interval": 256000,
                "history_lengths": [5, 15, 44, 130],
                "tag_widths": [9, 9, 10, 10]
            },
            "perceptron": {
                "history_length": 32,
                "table_bits": 10
            },
            "tournament": {
                "global_size_bits": 12,
                "local_hist_bits": 10,
                "local_pred_bits": 10
            }
        }
    }"#;

    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(config.memory.t_cas, 20);
    assert_eq!(config.memory.t_ras, 45);
    assert_eq!(config.memory.t_pre, 20);
    assert_eq!(config.memory.row_miss_latency, 200);
    assert_eq!(config.memory.tlb_size, 64);
}
