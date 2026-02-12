use crate::common::mocks::memory::{MockMemory, MockMemoryController};
use riscv_core::config::Config;
use riscv_core::core::Cpu;
use riscv_core::soc::System;
use riscv_core::soc::interconnect::Bus;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

pub struct TestContext {
    pub cpu: Cpu,
}

impl TestContext {
    pub fn new() -> Self {
        let _ = env_logger::builder().is_test(true).try_init();

        let config = Config::default();
        let bus = Bus::new(8, 0);

        let system = System {
            bus,
            mem_controller: Box::new(MockMemoryController::new(1)),
            exit_request: Arc::new(AtomicU64::new(u64::MAX)),
        };

        let mut cpu = Cpu::new(system, &config);

        // In tests, bypass the expensive simulate_memory_access path.
        // The default mmio_base == ram_base (0x8000_0000), which routes all
        // test-memory fetches through multi-cycle cache/DRAM simulation,
        // adding ~10 stall cycles per access and starving the pipeline.
        // Setting mmio_base to MAX ensures all addresses use the fast
        // bus transit time path instead.
        cpu.mmio_base = u64::MAX;

        Self { cpu }
    }

    pub fn with_memory(mut self, size: usize, base: u64) -> Self {
        let mem = MockMemory::new(size, base);
        self.cpu.bus.bus.add_device(Box::new(mem));
        self
    }

    /// Load a sequence of 32-bit instructions into memory at `addr` and set the PC.
    pub fn load_program(mut self, addr: u64, instructions: &[u32]) -> Self {
        for (i, inst) in instructions.iter().enumerate() {
            let offset = addr + (i as u64) * 4;
            self.cpu.bus.bus.write_u32(offset, *inst);
        }
        self.cpu.pc = addr;
        self
    }

    /// Set a general-purpose register value.
    pub fn set_reg(&mut self, reg: usize, val: u64) {
        self.cpu.regs.write(reg, val);
    }

    /// Read a general-purpose register value.
    pub fn get_reg(&self, reg: usize) -> u64 {
        self.cpu.regs.read(reg)
    }

    /// Run the CPU for a specific number of cycles.
    pub fn run(&mut self, cycles: u64) {
        for _ in 0..cycles {
            if let Err(e) = self.cpu.tick() {
                eprintln!("CPU tick error: {}", e);
                break;
            }
            if self.cpu.exit_code.is_some() {
                break;
            }
        }
    }
}
