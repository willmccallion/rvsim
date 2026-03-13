//! WebAssembly bindings for the RISC-V system simulator.
//!
//! Exposes the simulator to JavaScript via `wasm-bindgen`. Provides:
//! 1. **`WasmCpu`** — create, tick, step, run, and inspect the simulated CPU.
//! 2. **Configuration** — accepts a JS object matching the Python `Config.to_dict()` shape.
//! 3. **Statistics** — returns stats as a JS object after simulation.

// Relax lints for binding-layer code.
#![allow(
    missing_docs,
    missing_debug_implementations,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_value,
    clippy::uninlined_format_args,
    clippy::unused_self
)]

use wasm_bindgen::prelude::*;

mod conversion;

/// The simulation CPU, exposed to JavaScript.
///
/// Created via `WasmCpu::new(config, elf_bytes)`. All simulation state
/// lives inside the WASM linear memory.
#[wasm_bindgen]
pub struct WasmCpu {
    inner: rvsim_core::Simulator,
}

#[wasm_bindgen]
impl WasmCpu {
    /// Build a fully-configured CPU from a config object and ELF binary bytes.
    ///
    /// # Arguments
    ///
    /// * `config_js` - A JS object matching the shape of `Config.to_dict()`.
    /// * `elf_bytes` - Raw bytes of an ELF binary (bare-metal mode).
    #[wasm_bindgen(constructor)]
    pub fn new(config_js: JsValue, elf_bytes: &[u8]) -> Result<Self, JsError> {
        let config = conversion::js_to_config(config_js)?;
        let disk = String::new();
        let mut system = rvsim_core::soc::System::new(&config, &disk);

        // Load ELF binary.
        let result = rvsim_core::sim::loader::try_load_elf(elf_bytes, &mut system.bus)
            .ok_or_else(|| JsError::new("Not a valid ELF file"))?;

        if let Some(tohost) = result.tohost_addr {
            system.add_htif(tohost);
        }

        let mut sim = rvsim_core::Simulator::new(system, &config);
        sim.cpu.pc = result.entry;

        if let Some(tohost) = result.tohost_addr {
            sim.cpu.direct_mode = false;
            sim.cpu.privilege = rvsim_core::core::arch::mode::PrivilegeMode::Machine;
            sim.cpu.htif_range = Some((tohost, tohost + 16));
        }

        sim.sync_arch_regs();

        Ok(Self { inner: sim })
    }

    // ── Properties ───────────────────────────────────────────────────────────

    /// Program counter.
    #[wasm_bindgen(getter)]
    pub fn pc(&self) -> u64 {
        self.inner.cpu.pc
    }

    /// Set the program counter.
    #[wasm_bindgen(setter)]
    pub fn set_pc(&mut self, value: u64) {
        self.inner.cpu.pc = value;
    }

    /// Current privilege level: "M", "S", or "U".
    #[wasm_bindgen(getter)]
    pub fn privilege(&self) -> String {
        match self.inner.cpu.privilege {
            rvsim_core::core::arch::mode::PrivilegeMode::Machine => "M".into(),
            rvsim_core::core::arch::mode::PrivilegeMode::Supervisor => "S".into(),
            rvsim_core::core::arch::mode::PrivilegeMode::User => "U".into(),
        }
    }

    /// Total cycles simulated so far.
    #[wasm_bindgen(getter)]
    pub fn cycles(&self) -> u64 {
        self.inner.cpu.stats.cycles
    }

    /// Total instructions retired so far.
    #[wasm_bindgen(getter)]
    pub fn instructions_retired(&self) -> u64 {
        self.inner.cpu.stats.instructions_retired
    }

    /// Read a general-purpose register (0-31).
    pub fn read_reg(&self, idx: u8) -> u64 {
        self.inner.cpu.regs.read(rvsim_core::common::RegIdx::new(idx))
    }

    /// Write a general-purpose register (0-31).
    pub fn write_reg(&mut self, idx: u8, value: u64) {
        self.inner.cpu.regs.write(rvsim_core::common::RegIdx::new(idx), value);
    }

    // ── Simulation control ───────────────────────────────────────────────────

    /// Advance one clock cycle.
    pub fn tick(&mut self) -> Result<(), JsError> {
        self.inner.tick().map_err(|e| JsError::new(&e.to_string()))
    }

    /// Run until exit or cycle limit.
    ///
    /// Returns the exit code, or `undefined` if the limit was reached.
    /// Accepts `f64` because JS `number` (and Pyodide ints) don't auto-convert to `BigInt`.
    pub fn run(&mut self, limit: Option<f64>) -> Result<JsValue, JsError> {
        let limit = limit.map(|v| v as u64);
        let start = self.inner.cpu.stats.cycles;
        loop {
            if let Some(max) = limit
                && self.inner.cpu.stats.cycles.saturating_sub(start) >= max
            {
                return Ok(JsValue::UNDEFINED);
            }
            match self.inner.tick() {
                Ok(()) => {
                    if let Some(code) = self.inner.take_exit() {
                        return Ok(JsValue::from(code));
                    }
                }
                Err(e) => return Err(JsError::new(&e.to_string())),
            }
        }
    }

    /// Execute until one instruction commits.
    ///
    /// Returns `{ pc, raw, asm, cycles }` or `undefined` if the simulation
    /// exited before an instruction committed.
    pub fn step(&mut self, max_cycles: Option<f64>) -> Result<JsValue, JsError> {
        let max = max_cycles.map_or(100_000, |v| v as u64);
        let before_last = self.inner.cpu.pc_trace.last().copied();
        let mut cycles_run: u64 = 0;

        loop {
            if cycles_run >= max {
                return Ok(JsValue::UNDEFINED);
            }
            match self.inner.tick() {
                Ok(()) => {
                    if self.inner.take_exit().is_some() {
                        return Ok(JsValue::UNDEFINED);
                    }
                }
                Err(e) => return Err(JsError::new(&e.to_string())),
            }
            cycles_run += 1;

            let new_last = self.inner.cpu.pc_trace.last().copied();
            if new_last != before_last
                && let Some((pc, inst)) = new_last
            {
                let asm = rvsim_core::isa::disasm::disassemble(inst);
                let obj = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&obj, &"pc".into(), &JsValue::from(pc));
                let _ = js_sys::Reflect::set(&obj, &"raw".into(), &JsValue::from(inst));
                let _ = js_sys::Reflect::set(&obj, &"asm".into(), &JsValue::from(asm));
                let _ = js_sys::Reflect::set(
                    &obj,
                    &"cycles".into(),
                    &JsValue::from(self.inner.cpu.stats.cycles),
                );
                return Ok(obj.into());
            }
        }
    }

    /// Get simulation statistics as a JS object.
    pub fn stats(&self) -> Result<JsValue, JsError> {
        let s = &self.inner.cpu.stats;
        let obj = js_sys::Object::new();

        macro_rules! set {
            ($name:expr, $val:expr) => {
                let _ = js_sys::Reflect::set(&obj, &$name.into(), &JsValue::from($val));
            };
        }

        set!("cycles", s.cycles);
        set!("instructions_retired", s.instructions_retired);
        set!("icache_hits", s.icache_hits);
        set!("icache_misses", s.icache_misses);
        set!("dcache_hits", s.dcache_hits);
        set!("dcache_misses", s.dcache_misses);
        set!("l2_hits", s.l2_hits);
        set!("l2_misses", s.l2_misses);
        set!("l3_hits", s.l3_hits);
        set!("l3_misses", s.l3_misses);
        set!("stalls_mem", s.stalls_mem);
        set!("stalls_control", s.stalls_control);
        set!("stalls_data", s.stalls_data);
        set!("stalls_fu_structural", s.stalls_fu_structural);
        set!("stalls_backpressure", s.stalls_backpressure);
        set!("misprediction_penalty", s.misprediction_penalty);
        set!("pipeline_flushes", s.pipeline_flushes);
        set!("mem_ordering_violations", s.mem_ordering_violations);
        set!("branch_predictions", s.committed_branch_predictions);
        set!("branch_mispredictions", s.committed_branch_mispredictions);
        set!("traps_taken", s.traps_taken);

        let total_bp = s.committed_branch_predictions + s.committed_branch_mispredictions;
        let bp_acc = if total_bp > 0 {
            100.0 * (s.committed_branch_predictions as f64 / total_bp as f64)
        } else {
            0.0
        };
        set!("branch_accuracy_pct", bp_acc);

        let ipc = if s.cycles > 0 { s.instructions_retired as f64 / s.cycles as f64 } else { 0.0 };
        set!("ipc", ipc);

        set!("inst_load", s.inst_load);
        set!("inst_store", s.inst_store);
        set!("inst_branch", s.inst_branch);
        set!("inst_alu", s.inst_alu);
        set!("inst_system", s.inst_system);

        Ok(obj.into())
    }

    /// Disassemble a single 32-bit instruction.
    pub fn disassemble(inst: u32) -> String {
        rvsim_core::isa::disasm::disassemble(inst)
    }
}
