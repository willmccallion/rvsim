//! CPU Python binding.
//!
//! Exposes the full simulation interface as a single `Cpu` class. All properties
//! and methods that users interact with live here — nothing leaks through a Python
//! wrapper layer.

use crate::conversion::py_dict_to_config;
use crate::instruction::PyInstruction;
use crate::snapshot::PyPipelineSnapshot;
use crate::stats::PyStats;
use crate::views::{Csrs, Memory, Registers};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rvsim_core::Simulator;
use rvsim_core::core::arch::mode::PrivilegeMode;
use rvsim_core::sim::loader;
use std::io::Write;
use std::io::{BufReader, BufWriter, Read};

// ── Formatting helper ────────────────────────────────────────────────────────

fn fmt_commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len + len / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

// ── Cpu ──────────────────────────────────────────────────────────────────────

/// The simulation CPU. Created by `Simulator.build()`.
#[pyclass(name = "Cpu")]
pub struct PyCpu {
    pub inner: Simulator,
}

// ── Private Rust helpers (not exposed to Python) ─────────────────────────────

impl PyCpu {
    pub(crate) fn privilege_str(&self) -> &'static str {
        match self.inner.cpu.privilege {
            PrivilegeMode::Machine => "M",
            PrivilegeMode::Supervisor => "S",
            PrivilegeMode::User => "U",
        }
    }

    pub(crate) fn read_csr_by_name(&self, name: &str) -> Option<u64> {
        let c = &self.inner.cpu.csrs;
        match name {
            "mstatus" => Some(c.mstatus),
            "misa" => Some(c.misa),
            "mie" => Some(c.mie),
            "mip" => Some(c.mip),
            "mtvec" => Some(c.mtvec),
            "mepc" => Some(c.mepc),
            "mcause" => Some(c.mcause),
            "mtval" => Some(c.mtval),
            "medeleg" => Some(c.medeleg),
            "mideleg" => Some(c.mideleg),
            "mscratch" => Some(c.mscratch),
            "sstatus" => Some(c.sstatus),
            "sie" => Some(c.sie),
            "sip" => Some(c.sip),
            "stvec" => Some(c.stvec),
            "sepc" => Some(c.sepc),
            "scause" => Some(c.scause),
            "stval" => Some(c.stval),
            "sscratch" => Some(c.sscratch),
            "satp" => Some(c.satp),
            "cycle" => Some(c.cycle),
            "time" => Some(c.time),
            "instret" => Some(c.instret),
            "mcycle" => Some(c.mcycle),
            "minstret" => Some(c.minstret),
            "stimecmp" => Some(c.stimecmp),
            _ => None,
        }
    }

    /// Core run loop. Runs for up to `limit` cycles (or forever if `None`),
    /// checking Python signals every 10 000 cycles.
    fn run_inner(&mut self, py: Python<'_>, limit: Option<u64>) -> PyResult<Option<u64>> {
        let start = self.inner.cpu.stats.cycles;
        loop {
            if let Some(max) = limit
                && self.inner.cpu.stats.cycles.saturating_sub(start) >= max
            {
                let _ = std::io::stdout().flush();
                return Ok(None);
            }
            if self.inner.cpu.stats.cycles.is_multiple_of(10_000) {
                py.check_signals()?;
                let _ = std::io::stdout().flush();
            }
            match self.inner.tick() {
                Ok(_) => {
                    if let Some(code) = self.inner.take_exit() {
                        let _ = std::io::stdout().flush();
                        return Ok(Some(code));
                    }
                }
                Err(e) => return Err(PyRuntimeError::new_err(e)),
            }
        }
    }

    /// Run for exactly `cycles` cycles. Used by `run_until` and `sample`.
    fn run_for_cycles(&mut self, py: Python<'_>, cycles: u64) -> PyResult<Option<u64>> {
        self.run_inner(py, Some(cycles))
    }

    /// Run with stderr progress reporting every `progress` cycles.
    fn run_with_progress(
        &mut self,
        py: Python<'_>,
        limit: Option<u64>,
        progress: u64,
    ) -> PyResult<Option<u64>> {
        let mut cycles_run = 0u64;
        loop {
            let chunk = if let Some(max) = limit {
                let remaining = max.saturating_sub(cycles_run);
                if remaining == 0 {
                    eprint!("\r\x1b[2K");
                    let _ = std::io::stderr().flush();
                    return Ok(None);
                }
                progress.min(remaining)
            } else {
                progress
            };

            let exit = self.run_for_cycles(py, chunk)?;
            cycles_run += chunk;

            if let Some(code) = exit {
                eprint!("\r\x1b[2K");
                let _ = std::io::stderr().flush();
                return Ok(Some(code));
            }

            let s = &self.inner.cpu.stats;
            eprint!(
                "\r\x1b[36m[rvsim]\x1b[0m  {:>14} cycles  {:>14} insns",
                fmt_commas(s.cycles),
                fmt_commas(s.instructions_retired),
            );
            let _ = std::io::stderr().flush();
        }
    }
}

// ── Python-visible API ────────────────────────────────────────────────────────

#[pymethods]
impl PyCpu {
    /// Build a fully-configured CPU from a config dict and optional binary/kernel.
    ///
    /// This is the sole entry point for creating a Cpu. All system setup (ELF loading,
    /// HTIF registration, kernel loading) happens inside Rust — nothing leaks to Python.
    ///
    /// Args:
    ///     config_dict: The nested config dict (from ``Config.to_dict()``).
    ///     elf_data: Raw bytes of an ELF binary (bare-metal mode). Optional.
    ///     kernel_path: Path to a kernel image (kernel mode). Optional.
    ///     dtb_path: Path to a DTB file (kernel mode). Optional.
    ///     disk_path: Path to a disk image. Optional.
    #[new]
    #[pyo3(signature = (config_dict, *, elf_data=None, kernel_path=None, dtb_path=None, disk_path=None))]
    fn new(
        py: Python,
        config_dict: &Bound<'_, PyAny>,
        elf_data: Option<Vec<u8>>,
        kernel_path: Option<String>,
        dtb_path: Option<String>,
        disk_path: Option<String>,
    ) -> PyResult<Self> {
        let config = py_dict_to_config(py, config_dict)?;
        let disk = disk_path.unwrap_or_default();
        let mut system = rvsim_core::soc::System::new(&config, &disk);

        // ELF loading (bare-metal mode)
        let mut elf_entry: Option<u64> = None;
        let mut tohost_addr: Option<u64> = None;
        if let Some(data) = elf_data {
            if let Some(result) = loader::try_load_elf(&data, &mut system.bus) {
                elf_entry = Some(result.entry);
                if let Some(tohost) = result.tohost_addr {
                    system.add_htif(tohost);
                    tohost_addr = Some(tohost);
                }
            } else {
                return Err(PyRuntimeError::new_err(
                    "Not a valid ELF file. Only ELF binaries are supported.",
                ));
            }
        }

        let mut sim = Simulator::new(system, &config);

        // Apply ELF entry point
        if let Some(entry) = elf_entry {
            sim.cpu.pc = entry;
        }

        // HTIF setup (bare-metal with tohost symbol)
        if let Some(tohost) = tohost_addr {
            sim.cpu.direct_mode = false;
            sim.cpu.privilege = PrivilegeMode::Machine;
            sim.cpu.htif_range = Some((tohost, tohost + 16));
        }

        // Kernel loading
        if let Some(kpath) = kernel_path {
            loader::setup_kernel_load(&mut sim.cpu, &config, "", dtb_path, Some(kpath));
            sim.cpu.direct_mode = false;
        }

        Ok(PyCpu { inner: sim })
    }

    // ── Properties ───────────────────────────────────────────────────────────

    /// Program counter (read/write).
    #[getter]
    fn pc(&self) -> u64 {
        self.inner.cpu.pc
    }

    #[setter]
    fn set_pc(&mut self, value: u64) {
        self.inner.cpu.pc = value;
    }

    /// Current privilege level: ``"M"``, ``"S"``, or ``"U"`` (read-only).
    #[getter]
    fn privilege(&self) -> &'static str {
        self.privilege_str()
    }

    /// Whether instruction tracing is enabled (read/write).
    #[getter]
    fn trace(&self) -> bool {
        self.inner.cpu.trace
    }

    #[setter]
    fn set_trace(&mut self, value: bool) {
        self.inner.cpu.trace = value;
    }

    /// Performance statistics as a dict (read-only).
    #[getter]
    fn stats(&self, py: Python<'_>) -> PyResult<PyObject> {
        let s = PyStats::from(self.inner.cpu.stats.clone());
        Ok(s.to_dict(py)?.into_bound(py).into_any().unbind())
    }

    /// Register file — ``cpu.regs[10]``, ``cpu.regs[10] = v``.
    #[getter]
    fn regs(slf: Bound<'_, Self>) -> Registers {
        Registers { cpu: slf.unbind() }
    }

    /// CSR access — ``cpu.csrs["mstatus"]`` or ``cpu.csrs[0x300]``.
    #[getter]
    fn csrs(slf: Bound<'_, Self>) -> Csrs {
        Csrs { cpu: slf.unbind() }
    }

    /// Memory view for 32-bit reads — ``cpu.mem32[addr]``.
    #[getter]
    fn mem32(slf: Bound<'_, Self>) -> Memory {
        Memory {
            cpu: slf.unbind(),
            width: 32,
        }
    }

    /// Memory view for 64-bit reads — ``cpu.mem64[addr]``.
    #[getter]
    fn mem64(slf: Bound<'_, Self>) -> Memory {
        Memory {
            cpu: slf.unbind(),
            width: 64,
        }
    }

    /// Committed PC trace from the pipeline as a list of ``(pc, raw_inst)`` pairs.
    #[getter]
    fn pc_trace(&self) -> Vec<(u64, u32)> {
        self.inner.cpu.pc_trace.clone()
    }

    // ── Methods ──────────────────────────────────────────────────────────────

    /// Execute until one instruction commits.
    ///
    /// Returns an :class:`Instruction` or ``None`` if the simulation exited
    /// before an instruction could commit.
    #[pyo3(signature = (max_cycles=100_000))]
    fn step(&mut self, py: Python<'_>, max_cycles: u64) -> PyResult<Option<PyInstruction>> {
        let before_last = self.inner.cpu.pc_trace.last().copied();
        let mut cycles_run: u64 = 0;

        loop {
            if cycles_run >= max_cycles {
                return Ok(None);
            }
            if cycles_run.is_multiple_of(10_000) {
                py.check_signals()?;
            }
            match self.inner.tick() {
                Ok(_) => {
                    if self.inner.take_exit().is_some() {
                        return Ok(None);
                    }
                }
                Err(e) => return Err(PyRuntimeError::new_err(e)),
            }
            cycles_run += 1;

            let new_last = self.inner.cpu.pc_trace.last().copied();
            if new_last != before_last
                && let Some((pc, inst)) = new_last
            {
                let asm = rvsim_core::isa::disasm::disassemble(inst);
                return Ok(Some(PyInstruction {
                    pc,
                    raw: inst,
                    asm,
                    cycles: self.inner.cpu.stats.cycles,
                }));
            }
        }
    }

    /// Run the simulation until exit or cycle limit.
    ///
    /// Args:
    ///     limit: Max cycles to simulate. ``None`` means unlimited.
    ///     progress: Print progress to stderr every N cycles. 0 = silent.
    ///     stats_sections: Print stats on completion. ``None`` = suppress,
    ///         ``[]`` = all sections, ``["summary", ...]`` = specific sections.
    ///
    /// Returns:
    ///     Exit code or ``None`` if *limit* was reached without exiting.
    #[pyo3(signature = (limit=None, progress=0, stats_sections=None))]
    fn run(
        &mut self,
        py: Python<'_>,
        limit: Option<u64>,
        progress: u64,
        stats_sections: Option<Vec<String>>,
    ) -> PyResult<Option<u64>> {
        let exit = if progress > 0 {
            self.run_with_progress(py, limit, progress)?
        } else {
            self.run_inner(py, limit)?
        };

        if let Some(sections) = stats_sections {
            let s = PyStats::from(self.inner.cpu.stats.clone());
            if sections.is_empty() {
                s.print();
            } else {
                s.print_sections(sections);
            }
        }

        Ok(exit)
    }

    /// Run with periodic stats snapshots.
    ///
    /// Args:
    ///     every: Collect a stats snapshot every N cycles.
    ///     limit: Maximum total cycles. ``None`` runs until program exits.
    ///
    /// Returns:
    ///     List of stats dicts, one per interval. Wrap in ``Stats(s)`` for
    ///     ``.query()`` support.
    #[pyo3(signature = (every, limit=None))]
    fn sample(
        &mut self,
        py: Python<'_>,
        every: u64,
        limit: Option<u64>,
    ) -> PyResult<Vec<PyObject>> {
        let mut snapshots: Vec<PyObject> = Vec::new();
        let mut cycles_run = 0u64;

        loop {
            let chunk = if let Some(max) = limit {
                let remaining = max.saturating_sub(cycles_run);
                if remaining == 0 {
                    break;
                }
                every.min(remaining)
            } else {
                every
            };

            let exit = self.run_for_cycles(py, chunk)?;
            cycles_run += chunk;

            let s = PyStats::from(self.inner.cpu.stats.clone());
            snapshots.push(s.to_dict(py)?.into_bound(py).into_any().unbind());

            if exit.is_some() {
                break;
            }
        }

        Ok(snapshots)
    }

    /// Run until a predicate is satisfied or the simulation exits.
    ///
    /// Args:
    ///     predicate: ``lambda cpu: bool`` — stop when it returns ``True``.
    ///     pc: Stop when the program counter equals this value.
    ///     privilege: Stop when the privilege level equals this (``"M"``, ``"S"``, ``"U"``).
    ///     limit: Maximum total cycles (``None`` = unlimited).
    ///     chunk: Cycles between predicate checks.
    ///
    /// Returns:
    ///     Exit code if the simulation exited, or ``None`` if the condition was met
    ///     or *limit* was reached.
    #[pyo3(signature = (predicate=None, *, pc=None, privilege=None, limit=None, chunk=10_000))]
    fn run_until(
        slf: Bound<'_, PyCpu>,
        py: Python<'_>,
        predicate: Option<Py<PyAny>>,
        pc: Option<u64>,
        privilege: Option<String>,
        limit: Option<u64>,
        chunk: u64,
    ) -> PyResult<Option<u64>> {
        if predicate.is_none() && pc.is_none() && privilege.is_none() {
            return Err(PyRuntimeError::new_err(
                "run_until() requires at least one of: predicate, pc=, or privilege=",
            ));
        }

        let slf_py: Py<PyCpu> = slf.unbind();
        let mut cycles_run = 0u64;

        loop {
            let c = if let Some(max) = limit {
                let remaining = max.saturating_sub(cycles_run);
                if remaining == 0 {
                    return Ok(None);
                }
                chunk.min(remaining)
            } else {
                chunk
            };

            // Run a chunk, release borrow before checking predicates.
            let exit = slf_py.borrow_mut(py).run_for_cycles(py, c)?;
            cycles_run += c;

            if let Some(code) = exit {
                return Ok(Some(code));
            }

            // Check simple predicates with an immutable borrow.
            let stop = {
                let cpu = slf_py.borrow(py);
                pc.is_some_and(|p| cpu.inner.cpu.pc == p)
                    || privilege
                        .as_deref()
                        .is_some_and(|priv_str| cpu.privilege_str() == priv_str)
            };
            if stop {
                return Ok(None);
            }

            // Call Python predicate, passing the Cpu object itself.
            if let Some(ref pred) = predicate {
                let result = pred.call1(py, (slf_py.clone_ref(py),))?;
                if result.extract::<bool>(py)? {
                    return Ok(None);
                }
            }

            py.check_signals()?;
        }
    }

    /// Advance one cycle.
    fn tick(&mut self) -> PyResult<()> {
        self.inner.tick().map_err(PyRuntimeError::new_err)
    }

    /// Capture a snapshot of the current pipeline state.
    ///
    /// Returns a :class:`PipelineSnapshot` with the contents of every inter-stage
    /// latch as of the *end* of the last ``tick()``.  Call after ``tick()`` or
    /// ``step()`` to inspect what is currently in-flight.
    ///
    /// This performs a shallow clone of the latch vectors — it has no effect on
    /// simulation correctness or timing.
    fn pipeline_snapshot(&self) -> PyPipelineSnapshot {
        let width = self.inner.cpu.pipeline_width;
        PyPipelineSnapshot::new(self.inner.pipeline.snapshot(width))
    }

    /// Save a checkpoint of the full simulation state to a file.
    ///
    /// The checkpoint includes PC, registers, CSRs, privilege mode, and RAM.
    fn save(&self, path: &str) -> PyResult<()> {
        let cpu = &self.inner.cpu;
        let file = std::fs::File::create(path)
            .map_err(|e| PyRuntimeError::new_err(format!("cannot create checkpoint file: {e}")))?;
        let mut w = BufWriter::new(file);

        let mut header = serde_json::Map::new();
        header.insert("magic".into(), serde_json::Value::from("rvsim-checkpoint"));
        header.insert("version".into(), serde_json::Value::from(1u64));
        header.insert("pc".into(), serde_json::Value::from(cpu.pc));
        header.insert(
            "privilege".into(),
            serde_json::Value::from(cpu.privilege.to_u8()),
        );
        header.insert(
            "direct_mode".into(),
            serde_json::Value::from(cpu.direct_mode),
        );
        header.insert("trace".into(), serde_json::Value::from(cpu.trace));
        header.insert(
            "wfi_waiting".into(),
            serde_json::Value::from(cpu.wfi_waiting),
        );
        header.insert("wfi_pc".into(), serde_json::Value::from(cpu.wfi_pc));
        header.insert("ram_start".into(), serde_json::Value::from(cpu.ram_start));
        header.insert("ram_end".into(), serde_json::Value::from(cpu.ram_end));

        let gprs: Vec<serde_json::Value> = (0..32)
            .map(|i| serde_json::Value::from(cpu.regs.read(i)))
            .collect();
        header.insert("gpr".into(), serde_json::Value::Array(gprs));

        let fprs: Vec<serde_json::Value> = (0..32)
            .map(|i| serde_json::Value::from(cpu.regs.read_f(i)))
            .collect();
        header.insert("fpr".into(), serde_json::Value::Array(fprs));

        let c = &cpu.csrs;
        let mut csrs = serde_json::Map::new();
        csrs.insert("mstatus".into(), c.mstatus.into());
        csrs.insert("misa".into(), c.misa.into());
        csrs.insert("medeleg".into(), c.medeleg.into());
        csrs.insert("mideleg".into(), c.mideleg.into());
        csrs.insert("mie".into(), c.mie.into());
        csrs.insert("mtvec".into(), c.mtvec.into());
        csrs.insert("mscratch".into(), c.mscratch.into());
        csrs.insert("mepc".into(), c.mepc.into());
        csrs.insert("mcause".into(), c.mcause.into());
        csrs.insert("mtval".into(), c.mtval.into());
        csrs.insert("mip".into(), c.mip.into());
        csrs.insert("sstatus".into(), c.sstatus.into());
        csrs.insert("sie".into(), c.sie.into());
        csrs.insert("stvec".into(), c.stvec.into());
        csrs.insert("sscratch".into(), c.sscratch.into());
        csrs.insert("sepc".into(), c.sepc.into());
        csrs.insert("scause".into(), c.scause.into());
        csrs.insert("stval".into(), c.stval.into());
        csrs.insert("sip".into(), c.sip.into());
        csrs.insert("satp".into(), c.satp.into());
        csrs.insert("cycle".into(), c.cycle.into());
        csrs.insert("time".into(), c.time.into());
        csrs.insert("instret".into(), c.instret.into());
        csrs.insert("mcycle".into(), c.mcycle.into());
        csrs.insert("minstret".into(), c.minstret.into());
        csrs.insert("stimecmp".into(), c.stimecmp.into());
        csrs.insert("fflags".into(), c.fflags.into());
        csrs.insert("frm".into(), c.frm.into());
        csrs.insert("mcounteren".into(), c.mcounteren.into());
        csrs.insert("scounteren".into(), c.scounteren.into());
        header.insert("csrs".into(), serde_json::Value::Object(csrs));

        let header_bytes = serde_json::to_vec(&serde_json::Value::Object(header))
            .map_err(|e| PyRuntimeError::new_err(format!("serialization error: {e}")))?;
        let header_len = header_bytes.len() as u64;
        std::io::Write::write_all(&mut w, &header_len.to_le_bytes())
            .map_err(|e| PyRuntimeError::new_err(format!("write error: {e}")))?;
        std::io::Write::write_all(&mut w, &header_bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("write error: {e}")))?;

        let ram_size = (cpu.ram_end - cpu.ram_start) as usize;
        if !cpu.ram_ptr.is_null() && ram_size > 0 {
            let ram_slice = unsafe { std::slice::from_raw_parts(cpu.ram_ptr, ram_size) };
            std::io::Write::write_all(&mut w, ram_slice)
                .map_err(|e| PyRuntimeError::new_err(format!("write error: {e}")))?;
        }

        std::io::Write::flush(&mut w)
            .map_err(|e| PyRuntimeError::new_err(format!("flush error: {e}")))?;
        Ok(())
    }

    /// Restore simulation state from a checkpoint file.
    ///
    /// The CPU must have been created with compatible RAM size.
    fn restore(&mut self, path: &str) -> PyResult<()> {
        let file = std::fs::File::open(path)
            .map_err(|e| PyRuntimeError::new_err(format!("cannot open checkpoint file: {e}")))?;
        let mut r = BufReader::new(file);

        let mut len_buf = [0u8; 8];
        Read::read_exact(&mut r, &mut len_buf)
            .map_err(|e| PyRuntimeError::new_err(format!("read error: {e}")))?;
        let header_len = u64::from_le_bytes(len_buf) as usize;

        let mut header_bytes = vec![0u8; header_len];
        Read::read_exact(&mut r, &mut header_bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("read error: {e}")))?;
        let header: serde_json::Value = serde_json::from_slice(&header_bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("invalid checkpoint header: {e}")))?;

        let magic = header.get("magic").and_then(|v| v.as_str()).unwrap_or("");
        if magic != "rvsim-checkpoint" {
            return Err(PyRuntimeError::new_err("not a valid rvsim checkpoint file"));
        }

        let cpu = &mut self.inner.cpu;

        cpu.pc = header["pc"].as_u64().unwrap_or(0);
        cpu.privilege = PrivilegeMode::from_u8(header["privilege"].as_u64().unwrap_or(3) as u8);
        cpu.direct_mode = header["direct_mode"].as_bool().unwrap_or(false);
        cpu.trace = header["trace"].as_bool().unwrap_or(false);
        cpu.wfi_waiting = header["wfi_waiting"].as_bool().unwrap_or(false);
        cpu.wfi_pc = header["wfi_pc"].as_u64().unwrap_or(0);

        if let Some(gprs) = header["gpr"].as_array() {
            for (i, v) in gprs.iter().enumerate().take(32) {
                cpu.regs.write(i, v.as_u64().unwrap_or(0));
            }
        }

        if let Some(fprs) = header["fpr"].as_array() {
            for (i, v) in fprs.iter().enumerate().take(32) {
                cpu.regs.write_f(i, v.as_u64().unwrap_or(0));
            }
        }

        if let Some(csrs) = header.get("csrs") {
            let c = &mut cpu.csrs;
            macro_rules! restore_csr {
                ($field:ident) => {
                    if let Some(v) = csrs.get(stringify!($field)).and_then(|v| v.as_u64()) {
                        c.$field = v;
                    }
                };
            }
            restore_csr!(mstatus);
            restore_csr!(misa);
            restore_csr!(medeleg);
            restore_csr!(mideleg);
            restore_csr!(mie);
            restore_csr!(mtvec);
            restore_csr!(mscratch);
            restore_csr!(mepc);
            restore_csr!(mcause);
            restore_csr!(mtval);
            restore_csr!(mip);
            restore_csr!(sstatus);
            restore_csr!(sie);
            restore_csr!(stvec);
            restore_csr!(sscratch);
            restore_csr!(sepc);
            restore_csr!(scause);
            restore_csr!(stval);
            restore_csr!(sip);
            restore_csr!(satp);
            restore_csr!(cycle);
            restore_csr!(time);
            restore_csr!(instret);
            restore_csr!(mcycle);
            restore_csr!(minstret);
            restore_csr!(stimecmp);
            restore_csr!(fflags);
            restore_csr!(frm);
            restore_csr!(mcounteren);
            restore_csr!(scounteren);
        }

        let ckpt_ram_start = header["ram_start"].as_u64().unwrap_or(0);
        let ckpt_ram_end = header["ram_end"].as_u64().unwrap_or(0);
        let ckpt_ram_size = (ckpt_ram_end - ckpt_ram_start) as usize;
        let cpu_ram_size = (cpu.ram_end - cpu.ram_start) as usize;

        if ckpt_ram_size != cpu_ram_size {
            return Err(PyRuntimeError::new_err(format!(
                "RAM size mismatch: checkpoint has {} bytes, CPU has {} bytes",
                ckpt_ram_size, cpu_ram_size
            )));
        }

        if !cpu.ram_ptr.is_null() && ckpt_ram_size > 0 {
            let ram_slice = unsafe { std::slice::from_raw_parts_mut(cpu.ram_ptr, ckpt_ram_size) };
            Read::read_exact(&mut r, ram_slice)
                .map_err(|e| PyRuntimeError::new_err(format!("read error restoring RAM: {e}")))?;
        }

        cpu.l1_i_cache.flush();
        cpu.l1_d_cache.flush();
        cpu.l2_cache.flush();
        cpu.l3_cache.flush();
        cpu.mmu.dtlb.flush();
        cpu.mmu.itlb.flush();
        cpu.mmu.l2_tlb.flush();

        Ok(())
    }
}
