use crate::common::mocks::memory::MockMemory;
use rvsim_core::SimState;
use rvsim_core::Simulator;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::{Config, MemoryController};
use rvsim_core::isa::reg::RegIdx;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

pub struct TestContext {
    pub sim: Simulator,
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TestContext {
    pub fn new() -> Self {
        Self::new_with_config(&Config::default())
    }

    /// Construct a TestContext with a caller-supplied `Config` (e.g. to vary
    /// pipeline width or backend type when reproducing pipeline-integration
    /// bugs). Otherwise identical to `new()`.
    pub fn new_with_config(config: &Config) -> Self {
        let _ = env_logger::builder().is_test(true).try_init();

        // Pipeline tests exercise stage behaviour, not the memory system:
        // give every miss a single-cycle memory controller and a zero-latency
        // bus so short programs finish inside their cycle budgets.
        let mut config = config.clone();
        config.memory.controller = MemoryController::Simple;
        config.memory.row_miss_latency = 1;
        config.system.bus_latency = 0;

        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        let state = rvsim_core::SimState::new(&config, "", exit_signal);
        let sim = Simulator::new(state);

        Self { sim }
    }

    /// Convenience accessor for the CPU.
    pub fn cpu(&self) -> &SimState {
        &self.sim.state
    }

    /// Mutable convenience accessor for the CPU.
    pub fn cpu_mut(&mut self) -> &mut SimState {
        &mut self.sim.state
    }

    pub fn with_memory(mut self, size: usize, base: u64) -> Self {
        let mem = MockMemory::new(size, base);
        self.sim.state.bus.add_device(Box::new(mem));
        self
    }

    /// Load a sequence of 32-bit instructions into memory at `addr` and set the PC.
    pub fn load_program(mut self, addr: u64, instructions: &[u32]) -> Self {
        for (i, inst) in instructions.iter().enumerate() {
            let offset = addr + (i as u64) * 4;
            self.sim.probe_mem_store(PhysAddr::new(offset), u64::from(*inst), 4);
        }
        self.sim.set_pc(0, addr);
        self
    }

    /// Set a general-purpose register value.
    pub fn set_reg(&mut self, reg: usize, val: u64) {
        self.sim.state.harts[0].regs.write(RegIdx::new(reg as u8), val);
    }

    /// Read a general-purpose register value.
    pub fn get_reg(&self, reg: usize) -> u64 {
        self.sim.state.harts[0].regs.read(RegIdx::new(reg as u8))
    }

    /// Ticks until `done` holds and returns the cycle it first held, or
    /// `None` if it never held within `max_cycles`.
    pub fn run_until(&mut self, max_cycles: u64, done: impl Fn(&Self) -> bool) -> Option<u64> {
        for _ in 0..max_cycles {
            if done(self) {
                return Some(self.sim.state.cycle);
            }
            if let Err(e) = self.sim.tick() {
                eprintln!("CPU tick error: {}", e);
                return None;
            }
        }
        None
    }

    /// Run the CPU for a specific number of cycles.
    pub fn run(&mut self, cycles: u64) {
        for _ in 0..cycles {
            if let Err(e) = self.sim.tick() {
                eprintln!("CPU tick error: {}", e);
                break;
            }
            if self.sim.state.check_exit().is_some() {
                break;
            }
        }
    }
}
