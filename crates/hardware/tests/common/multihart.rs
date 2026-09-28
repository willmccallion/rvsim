//! A system with several harts running one bare-metal program in direct
//! mode, for tests of cross-hart memory behaviour.

use rvsim_core::Simulator;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::Config;
use rvsim_core::core::pipeline::engine::BackendType;

/// Where multi-hart test programs are loaded; every hart starts here.
pub const PROGRAM_BASE: u64 = 0x8000_0000;

/// Scratch data area the programs use, 1 KiB above the code.
pub const DATA_BASE: u64 = PROGRAM_BASE + 0x400;

pub struct MultiHart {
    pub sim: Simulator,
}

impl MultiHart {
    /// Builds a `hart_count`-hart system on `backend` with the program
    /// loaded at [`PROGRAM_BASE`] and every hart's PC pointing at it.
    pub fn new(hart_count: usize, backend: BackendType, program: &[u32]) -> Self {
        let mut config = Config::default();
        config.system.hart_count = hart_count;
        config.system.console = rvsim_core::config::Console::Quiet;
        config.pipeline.backend = backend;
        let mut sim = Simulator::build(&config, "");
        for (index, word) in program.iter().enumerate() {
            let addr = PROGRAM_BASE + (index as u64) * 4;
            sim.probe_mem_store(PhysAddr::new(addr), u64::from(*word), 4);
        }
        for hart in &mut sim.state.harts {
            hart.pc = PROGRAM_BASE;
        }
        sim.sync_arch_regs();
        Self { sim }
    }

    /// Ticks until a hart exits or `max_cycles` elapse; returns the exit code.
    pub fn run_until_exit(&mut self, max_cycles: u64) -> Option<u64> {
        for _ in 0..max_cycles {
            self.sim.tick().expect("tick");
            if let Some(code) = self.sim.state.check_exit() {
                return Some(code);
            }
        }
        None
    }

    pub fn read_u64(&mut self, addr: u64) -> u64 {
        self.sim.probe_mem_load(PhysAddr::new(addr), 8)
    }
}

impl MultiHart {
    /// Like [`MultiHart::new`], with the caller's configuration (its hart
    /// count included).
    pub fn with_config(config: &Config, program: &[u32]) -> Self {
        let mut sim = Simulator::build(config, "");
        for (index, word) in program.iter().enumerate() {
            let addr = PROGRAM_BASE + (index as u64) * 4;
            sim.probe_mem_store(PhysAddr::new(addr), u64::from(*word), 4);
        }
        for hart in &mut sim.state.harts {
            hart.pc = PROGRAM_BASE;
        }
        sim.sync_arch_regs();
        Self { sim }
    }
}
