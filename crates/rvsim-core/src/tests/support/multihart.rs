//! A system with several harts running one bare-metal program in direct
//! mode, for tests of cross-hart memory behaviour.

use crate::Simulator;
use crate::common::{PhysAddr, VirtAddr};
use crate::config::BackendKind;
use crate::config::Config;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::harness::build_simulator;

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
    pub fn new(hart_count: usize, backend: BackendKind, program: &[u32]) -> Self {
        let mut config = Config::minimal();
        config.system.hart_count = std::num::NonZero::new(hart_count).unwrap();
        config.system.console = crate::config::Console::Quiet;
        config.pipeline.backend = backend;
        let mut sim = build_simulator(&config);
        for (index, word) in program.iter().enumerate() {
            let addr = PROGRAM_BASE + (index as u64) * 4;
            sim.probe_mem_store(PhysAddr::new(addr), u64::from(*word), ProbeWidth::B4);
        }
        for hart in &mut sim.state.harts {
            hart.pc = VirtAddr::new(PROGRAM_BASE);
        }
        sim.sync_arch_regs();
        Self { sim }
    }

    /// Ticks until a hart exits or `max_cycles` elapse; returns the exit code.
    pub fn run_until_exit(&mut self, max_cycles: u64) -> Option<u64> {
        for _ in 0..max_cycles {
            self.sim.tick().expect("tick");
            if let Some(code) = self.sim.state.uncore.peek_exit() {
                return Some(code);
            }
        }
        None
    }

    pub fn read_u64(&mut self, addr: u64) -> u64 {
        self.sim.probe_mem_load(PhysAddr::new(addr), ProbeWidth::B8)
    }
}

impl MultiHart {
    /// Like [`MultiHart::new`], with the caller's configuration (its hart
    /// count included).
    pub fn with_config(config: &Config, program: &[u32]) -> Self {
        let mut sim = build_simulator(config);
        for (index, word) in program.iter().enumerate() {
            let addr = PROGRAM_BASE + (index as u64) * 4;
            sim.probe_mem_store(PhysAddr::new(addr), u64::from(*word), ProbeWidth::B4);
        }
        for hart in &mut sim.state.harts {
            hart.pc = VirtAddr::new(PROGRAM_BASE);
        }
        sim.sync_arch_regs();
        Self { sim }
    }
}
