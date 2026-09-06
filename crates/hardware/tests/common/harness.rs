use crate::common::mocks::memory::MockMemory;
use rvsim_core::Simulator;
use rvsim_core::common::{PhysAddr, RegIdx};
use rvsim_core::config::Config;
use rvsim_core::SimState;
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

        let exit_signal = Arc::new(AtomicU64::new(u64::MAX));
        let cpu = rvsim_core::SimState::new(config, "", exit_signal);
        let mut sim = Simulator::new(cpu);

        // Bypass cache simulation in tests: default cache_base == ram_base routes
        // every access through multi-cycle DRAM, starving the pipeline.
        sim.state.config.system.ram_base = u64::MAX;

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
        self.sim.state.hart.pc = addr;
        self
    }

    /// Set a general-purpose register value.
    pub fn set_reg(&mut self, reg: usize, val: u64) {
        self.sim.state.hart.regs.write(RegIdx::new(reg as u8), val);
    }

    /// Read a general-purpose register value.
    pub fn get_reg(&self, reg: usize) -> u64 {
        self.sim.state.hart.regs.read(RegIdx::new(reg as u8))
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
