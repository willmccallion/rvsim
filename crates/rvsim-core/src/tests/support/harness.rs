use crate::Simulator;
use crate::common::{PhysAddr, VirtAddr};
use crate::config::{Config, MemoryControllerKind, ValidConfig};
use crate::isa::csr::{MSTATUS_FS_INIT, MSTATUS_VS_INIT};
use crate::isa::reg::RegIdx;
use crate::soc::exit::ExitSignal;
use crate::system::SystemState;
use crate::system::simulator::ProbeWidth;
use crate::tests::support::mocks::memory::MockMemory;

/// A simulator for `config` with no disk.
pub fn build_simulator(config: &Config) -> Simulator {
    Simulator::build(&valid(config), None).expect("the test machine builds")
}

/// `config`, which the test expects [`Config::validate`] to accept.
pub fn valid(config: &Config) -> ValidConfig {
    ValidConfig::try_from(config.clone()).expect("the test config is valid")
}

pub struct TestContext {
    pub sim: Simulator,
}

impl Default for TestContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TestContext {
    /// The minimal machine, with fast memory.
    pub fn new() -> Self {
        Self::with_fast_memory(&Config::minimal())
    }

    /// The machine `config` describes, except that its memory is a
    /// one-cycle controller behind a zero-latency bus, so short programs
    /// finish inside their cycle budgets whatever the memory config says.
    pub fn with_fast_memory(config: &Config) -> Self {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        let mut config = config.clone();
        config.memory.controller = MemoryControllerKind::Simple;
        config.memory.simple_latency = 1;
        config.system.bus_latency = 0;

        let exit_signal = ExitSignal::new();
        let mut state =
            SystemState::new(&valid(&config), None, exit_signal).expect("the test machine builds");
        // What a bare-metal program's start-up does before main: switch the
        // FPU and the vector unit on, so test programs can use them.
        for hart in &mut state.harts {
            hart.csrs.mstatus |= MSTATUS_FS_INIT | MSTATUS_VS_INIT;
        }
        let sim = Simulator::new(state);

        Self { sim }
    }

    /// The system the test runs.
    pub fn state(&self) -> &SystemState {
        &self.sim.state
    }

    /// The system the test runs, to change.
    pub fn state_mut(&mut self) -> &mut SystemState {
        &mut self.sim.state
    }

    pub fn with_memory(mut self, size: usize, base: u64) -> Self {
        let mem = MockMemory::new(size, base);
        self.sim.state.uncore.bus.add_device(Box::new(mem));
        self
    }

    /// Load a sequence of 32-bit instructions into memory at `addr` and set the PC.
    pub fn load_program(mut self, addr: u64, instructions: &[u32]) -> Self {
        for (i, inst) in instructions.iter().enumerate() {
            let offset = addr + (i as u64) * 4;
            self.sim.probe_mem_store(PhysAddr::new(offset), u64::from(*inst), ProbeWidth::B4);
        }
        self.sim.set_pc(0, VirtAddr::new(addr));
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
                return Some(self.sim.state.uncore.kernel.cycle);
            }
            self.tick();
        }
        None
    }

    /// Run the CPU for a specific number of cycles.
    pub fn run(&mut self, cycles: u64) {
        for _ in 0..cycles {
            self.tick();
            if self.sim.state.uncore.peek_exit().is_some() {
                break;
            }
        }
    }

    /// Advances one cycle, failing the test at the cycle a simulator error occurs.
    fn tick(&mut self) {
        if let Err(e) = self.sim.tick() {
            panic!("tick failed at cycle {}: {e}", self.sim.state.uncore.kernel.cycle);
        }
    }
}
