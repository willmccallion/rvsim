//! Simulator: owns both the CPU and the pipeline side-by-side.
//!
//! This avoids the borrow-splitting hack where the pipeline was stored as
//! `Option<PipelineDispatch>` inside `Cpu` and temporarily `take()`-en each tick.

use crate::common::SimError;
use crate::config::Config;
use crate::core::Cpu;
use crate::core::pipeline::backend::inorder::InOrderEngine;
use crate::core::pipeline::backend::o3::O3Engine;
use crate::core::pipeline::engine::{BackendType, Pipeline, PipelineDispatch};
use crate::core::pipeline::frontend::Frontend;
use crate::soc::System;

/// Top-level simulator: CPU architectural state + pipeline.
#[derive(Debug)]
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
        let pipeline = match config.pipeline.backend {
            BackendType::InOrder => PipelineDispatch::InOrder(Box::new(Pipeline {
                frontend: Frontend::new(config.pipeline.width),
                engine: InOrderEngine::new(config),
                rename_output: Vec::with_capacity(config.pipeline.width),
            })),
            BackendType::OutOfOrder => PipelineDispatch::OutOfOrder(Box::new(Pipeline {
                frontend: Frontend::new(config.pipeline.width),
                engine: O3Engine::new(config),
                rename_output: Vec::with_capacity(config.pipeline.width),
            })),
        };
        Self { cpu, pipeline }
    }

    /// Synchronize the architectural register file into the O3 PRF.
    ///
    /// Must be called after all register initialization (loader setup, etc.)
    /// but before the first pipeline tick. For the in-order backend this is a no-op.
    pub fn sync_arch_regs(&mut self) {
        if let PipelineDispatch::OutOfOrder(ref mut p) = self.pipeline {
            p.engine.sync_arch_regs(&self.cpu);
        }
    }

    /// Advances the simulator by one clock cycle.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::HangDetected`] if the PC has not advanced for too many
    /// consecutive cycles (and is not stuck in a WFI spin-wait).
    ///
    /// Returns [`SimError::KernelPanic`] if the guest OS panic sentinel fires.
    pub fn tick(&mut self) -> Result<(), SimError> {
        let prev_priv = self.cpu.privilege;
        let skip = self.cpu.pre_tick()?;
        if !skip {
            self.pipeline.tick(&mut self.cpu);
        }
        self.cpu.post_tick(prev_priv);
        Ok(())
    }

    /// Retrieves the exit code if the simulation has finished.
    pub const fn take_exit(&mut self) -> Option<u64> {
        self.cpu.take_exit()
    }
}
