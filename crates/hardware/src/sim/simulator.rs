//! Simulator: owns both the CPU and the pipeline side-by-side.
//!
//! This avoids the borrow-splitting hack where the pipeline was stored as
//! `Option<PipelineDispatch>` inside `Cpu` and temporarily `take()`-en each tick.

use crate::config::Config;
use crate::core::Cpu;
use crate::core::pipeline::backend::inorder::InOrderEngine;
use crate::core::pipeline::engine::{Pipeline, PipelineDispatch};
use crate::core::pipeline::frontend::Frontend;
use crate::soc::System;

/// Top-level simulator: CPU architectural state + pipeline.
pub struct Simulator {
    /// CPU architectural state (registers, caches, MMU, bus, stats).
    pub cpu: Cpu,
    /// Pipeline implementation (frontend + backend engine).
    pub pipeline: PipelineDispatch,
}

unsafe impl Send for Simulator {}
unsafe impl Sync for Simulator {}

impl Simulator {
    /// Creates a new simulator with the given system and configuration.
    pub fn new(system: System, config: &Config) -> Self {
        let cpu = Cpu::new(system, config);
        let pipeline = PipelineDispatch::InOrder(Box::new(Pipeline {
            frontend: Frontend::new(config.pipeline.width),
            engine: InOrderEngine::new(config),
            rename_output: Vec::with_capacity(config.pipeline.width),
        }));
        Self { cpu, pipeline }
    }

    /// Advances the simulator by one clock cycle.
    pub fn tick(&mut self) -> Result<(), String> {
        let prev_priv = self.cpu.privilege;
        let skip = self.cpu.pre_tick()?;
        if !skip {
            self.pipeline.tick(&mut self.cpu);
        }
        self.cpu.post_tick(prev_priv);
        Ok(())
    }

    /// Retrieves the exit code if the simulation has finished.
    pub fn take_exit(&mut self) -> Option<u64> {
        self.cpu.take_exit()
    }
}
