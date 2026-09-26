//! CPU Python binding.
//!
//! Exposes the full simulation interface as a single `Cpu` class. All properties
//! and methods that users interact with live here — nothing leaks through a Python
//! wrapper layer.

use crate::conversion::py_dict_to_config;
use crate::instruction::PyInstruction;
use crate::snapshot::PyPipelineSnapshot;
use crate::stats::PyStats;
use crate::views::{Csrs, Harts, Memory, Registers, VirtualMemory};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rvsim_core::Simulator;
use rvsim_core::common::{CsrAddr, HartId};
use rvsim_core::core::arch::mode::PrivilegeMode;
use rvsim_core::sim::loader;
use std::io::Write;
use std::io::{BufReader, BufWriter, Read};

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

/// The running simulator.
#[pyclass(name = "Simulator", subclass)]
pub struct PySimulator {
    pub inner: Simulator,
}

impl PySimulator {
    pub(crate) fn privilege_str(&self, hart: usize) -> &'static str {
        match self.inner.state.harts[hart].privilege {
            PrivilegeMode::Machine => "M",
            PrivilegeMode::Supervisor => "S",
            PrivilegeMode::User => "U",
        }
    }

    /// Reads `addr` on `hart` the way a CSR instruction would; `None`
    /// when the hart does not implement that CSR.
    pub(crate) fn read_csr(&mut self, hart: usize, addr: CsrAddr) -> Option<u64> {
        let core = self.inner.state.topology.core_of_hart(HartId::new(hart as u32))?;
        let ctx = self.inner.state.core_ctx(core.as_index());
        ctx.is_valid_csr(addr).then(|| ctx.csr_read(addr))
    }

    /// Runs for up to `limit` cycles, checking Python signals every 10000 cycles.
    fn run_inner(&mut self, py: Python<'_>, limit: Option<u64>) -> PyResult<Option<u64>> {
        let start = self.inner.state.cycle;
        loop {
            if let Some(max) = limit
                && self.inner.state.cycle.saturating_sub(start) >= max
            {
                let _ = std::io::stdout().flush();
                return Ok(None);
            }
            if self.inner.state.cycle.is_multiple_of(10_000) {
                py.check_signals()?;
                let _ = std::io::stdout().flush();
            }
            match self.inner.tick() {
                Ok(()) => {
                    if let Some(code) = self.inner.take_exit() {
                        let _ = std::io::stdout().flush();
                        return Ok(Some(code));
                    }
                }
                Err(e) => return Err(PyRuntimeError::new_err(e.to_string())),
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

            eprint!(
                "\r\x1b[36m[rvsim]\x1b[0m  {:>14} cycles  {:>14} insns",
                fmt_commas(self.inner.state.cycle),
                fmt_commas(self.inner.state.instructions_retired()),
            );
            let _ = std::io::stderr().flush();
        }
    }
}

#[pymethods]
impl PySimulator {
    /// Build a fully-configured CPU from a config dict and optional binary/kernel.
    ///
    /// This is the sole entry point for creating a Cpu. All system setup (ELF loading,
    /// HTIF registration, kernel loading) happens inside Rust — nothing leaks to Python.
    ///
    /// Args:
    ///     `config_dict`: The nested config dict (from ``Config.to_dict()``).
    ///     `elf_data`: Raw bytes of an ELF binary (bare-metal mode). Optional.
    ///     `kernel_path`: Path to a kernel image (kernel mode). Optional.
    ///     `dtb_path`: Path to a DTB file (kernel mode). Optional.
    ///     `disk_path`: Path to a disk image. Optional.
    #[new]
    #[pyo3(signature = (config_dict, *, elf_data=None, kernel_path=None, dtb_path=None, disk_path=None))]
    fn new(
        py: Python<'_>,
        config_dict: &Bound<'_, PyAny>,
        elf_data: Option<Vec<u8>>,
        kernel_path: Option<String>,
        dtb_path: Option<String>,
        disk_path: Option<String>,
    ) -> PyResult<Self> {
        let config = py_dict_to_config(py, config_dict)?;
        config.validate().map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let disk = disk_path.unwrap_or_default();
        let exit_signal = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
        let mut cpu = rvsim_core::SimState::new(&config, &disk, exit_signal.clone());

        let mut elf_entry: Option<u64> = None;
        let mut tohost_addr: Option<u64> = None;
        if let Some(data) = elf_data {
            if let Some(result) = loader::try_load_elf(&data, &mut cpu.bus) {
                elf_entry = Some(result.entry);
                if let Some(tohost) = result.tohost_addr {
                    cpu.add_htif(tohost, &exit_signal);
                    tohost_addr = Some(tohost);
                }
            } else {
                return Err(PyRuntimeError::new_err(
                    "Not a valid ELF file. Only ELF binaries are supported.",
                ));
            }
        }

        let mut sim = Simulator::new(cpu);

        if let Some(entry) = elf_entry {
            sim.set_pc(0, entry);
        }

        if tohost_addr.is_some() {
            sim.state.direct_mode = false;
            sim.state.harts[0].privilege = PrivilegeMode::Machine;
        }

        if let Some(kpath) = kernel_path {
            loader::setup_kernel_load(&mut sim.state, &config, "", dtb_path, Some(kpath))
                .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
            sim.state.direct_mode = false;
        }

        // Sync arch regs into the O3 PRF — must happen after all reg init.
        sim.sync_arch_regs();

        Ok(Self { inner: sim })
    }

    /// The architectural PC (the next instruction to retire). Writing it
    /// drops everything in flight and restarts fetch there.
    #[getter]
    fn pc(&self) -> u64 {
        self.inner.state.harts[0].pc
    }

    #[setter]
    fn set_pc(&mut self, value: u64) {
        self.inner.set_pc(0, value);
    }

    /// Current privilege level: ``"M"``, ``"S"``, or ``"U"`` (read-only).
    #[getter]
    fn privilege(&self) -> &'static str {
        self.privilege_str(0)
    }

    /// Whether instruction tracing is armed (read/write). Events go to
    /// stderr through the ``RUST_LOG`` filter (``rvsim::trap=trace``,
    /// ``rvsim::fetch=trace``, ...), each tagged with the hart it belongs
    /// to; :meth:`trace_filter` narrows them further.
    #[getter]
    fn trace(&self) -> bool {
        self.inner.state.trace.armed
    }

    #[setter]
    fn set_trace(&mut self, value: bool) {
        self.inner.state.trace.armed = value;
    }

    /// Narrows what an armed trace prints.
    ///
    /// Args:
    ///     harts: Hart ids whose events print; ``None`` for every hart.
    ///     cycles: ``(first, last)`` cycle window; ``None`` for no window.
    ///     `trap_causes`: `mcause` values (interrupt bit included) whose
    ///         trap-taken events print; ``None`` prints every trap except
    ///         timer interrupts and ecalls.
    #[pyo3(signature = (harts=None, cycles=None, trap_causes=None))]
    fn trace_filter(
        &mut self,
        harts: Option<Vec<u32>>,
        cycles: Option<(u64, u64)>,
        trap_causes: Option<Vec<u64>>,
    ) {
        let trace = &mut self.inner.state.trace;
        trace.harts = harts.unwrap_or_default().into_iter().map(HartId::new).collect();
        trace.cycle_from = cycles.map(|(from, _)| from);
        trace.cycle_to = cycles.map(|(_, to)| to);
        trace.trap_causes = trap_causes.unwrap_or_default();
    }

    /// Snapshot of the stats tree — path lookups, wildcard queries, and the
    /// auto-summary. See [`crate::stats::PyStats`].
    #[getter]
    fn stats(&self) -> PyStats {
        PyStats::new(
            self.inner.state.stats.clone(),
            self.inner.state.cycle,
            self.inner.state.instructions_retired(),
        )
    }

    /// Hart 0's register file — ``cpu.regs[10]``, ``cpu.regs[10] = v``.
    #[getter]
    fn regs(slf: Bound<'_, Self>) -> Registers {
        Registers { cpu: slf.unbind(), hart: 0 }
    }

    /// Hart 0's CSRs — ``cpu.csrs["mstatus"]`` or ``cpu.csrs[0x300]``.
    #[getter]
    fn csrs(slf: Bound<'_, Self>) -> Csrs {
        Csrs { cpu: slf.unbind(), hart: 0 }
    }

    /// Every hart — ``cpu.harts[1].pc``, ``cpu.harts[1].regs[10]``,
    /// ``len(cpu.harts)``.
    #[getter]
    fn harts(slf: Bound<'_, Self>) -> Harts {
        Harts { cpu: slf.unbind() }
    }

    /// Number of harts in the system.
    #[getter]
    fn hart_count(&self) -> usize {
        self.inner.state.harts.len()
    }

    /// Memory view for 32-bit reads — ``cpu.mem32[addr]``.
    #[getter]
    fn mem32(slf: Bound<'_, Self>) -> Memory {
        Memory { cpu: slf.unbind(), width: 32 }
    }

    /// Memory view for 64-bit reads — ``cpu.mem64[addr]``.
    #[getter]
    fn mem64(slf: Bound<'_, Self>) -> Memory {
        Memory { cpu: slf.unbind(), width: 64 }
    }

    /// Virtual memory view for 32-bit reads — ``cpu.vmem32[vaddr]``.
    ///
    /// Translates the virtual address through the current page tables (SATP)
    /// before reading. Raises ``ValueError`` if translation fails.
    #[getter]
    fn vmem32(slf: Bound<'_, Self>) -> VirtualMemory {
        VirtualMemory { cpu: slf.unbind(), width: 32 }
    }

    /// Virtual memory view for 64-bit reads — ``cpu.vmem64[vaddr]``.
    ///
    /// Translates the virtual address through the current page tables (SATP)
    /// before reading. Raises ``ValueError`` if translation fails.
    #[getter]
    fn vmem64(slf: Bound<'_, Self>) -> VirtualMemory {
        VirtualMemory { cpu: slf.unbind(), width: 64 }
    }

    /// Committed PC trace from the pipeline as a list of ``(pc, raw_inst)`` pairs.
    #[getter]
    fn pc_trace(&self) -> Vec<(u64, u32)> {
        self.inner.state.per_hart_debug[0].pc_trace.clone()
    }

    /// Open a commit log file. Each retired instruction is written as
    /// ``core   0: 0x<pc> (0x<inst>)``. Requires the ``commit-log`` feature.
    #[cfg(feature = "commit-log")]
    fn open_commit_log(&mut self, path: &str) -> PyResult<()> {
        self.inner.state.open_commit_log(path).map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Execute until one instruction commits.
    ///
    /// Returns an :class:`Instruction` or ``None`` if the simulation exited
    /// before an instruction could commit.
    #[pyo3(signature = (max_cycles=100_000))]
    fn step(&mut self, py: Python<'_>, max_cycles: u64) -> PyResult<Option<PyInstruction>> {
        let before_last = self.inner.state.per_hart_debug[0].pc_trace.last().copied();
        let mut cycles_run: u64 = 0;

        loop {
            if cycles_run >= max_cycles {
                return Ok(None);
            }
            if cycles_run.is_multiple_of(10_000) {
                py.check_signals()?;
            }
            match self.inner.tick() {
                Ok(()) => {
                    if self.inner.take_exit().is_some() {
                        return Ok(None);
                    }
                }
                Err(e) => return Err(PyRuntimeError::new_err(e.to_string())),
            }
            cycles_run += 1;

            let new_last = self.inner.state.per_hart_debug[0].pc_trace.last().copied();
            if new_last != before_last
                && let Some((pc, inst)) = new_last
            {
                let asm = rvsim_core::isa::disasm::disassemble(inst);
                return Ok(Some(PyInstruction {
                    pc,
                    raw: inst,
                    asm,
                    cycles: self.inner.state.cycle,
                }));
            }
        }
    }

    /// Run the simulation until exit or cycle limit.
    ///
    /// Args:
    ///     limit: Max cycles to simulate. ``None`` means unlimited.
    ///     progress: Print progress to stderr every N cycles. 0 = silent.
    ///     `stats_sections`: Print stats on completion. ``None`` suppresses
    ///         the report; ``[]`` prints all subjects; a list of subjects
    ///         (e.g. ``["core0", "hart0"]``) restricts output to those
    ///         subjects. Use :meth:`Stats.subjects` to enumerate.
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
            let text = if sections.is_empty() {
                self.inner
                    .state
                    .stats
                    .summary(self.inner.state.cycle, self.inner.state.instructions_retired())
            } else {
                let refs: Vec<&str> = sections.iter().map(String::as_str).collect();
                self.inner.state.stats.summary_sections(
                    self.inner.state.cycle,
                    self.inner.state.instructions_retired(),
                    &refs,
                )
            };
            println!("{text}");
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
    ///     List of :class:`Stats` snapshots, one per interval.
    #[pyo3(signature = (every, limit=None))]
    fn sample(&mut self, py: Python<'_>, every: u64, limit: Option<u64>) -> PyResult<Vec<PyStats>> {
        let mut snapshots: Vec<PyStats> = Vec::new();
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

            snapshots.push(PyStats::new(
                self.inner.state.stats.clone(),
                self.inner.state.cycle,
                self.inner.state.instructions_retired(),
            ));

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
        slf: Bound<'_, Self>,
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

        let slf_py: Py<Self> = slf.unbind();
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

            let exit = slf_py.borrow_mut(py).run_for_cycles(py, c)?;
            cycles_run += c;

            if let Some(code) = exit {
                return Ok(Some(code));
            }

            let stop = {
                let cpu = slf_py.borrow(py);
                pc.is_some_and(|p| cpu.inner.state.harts[0].pc == p)
                    || privilege.as_deref().is_some_and(|priv_str| cpu.privilege_str(0) == priv_str)
            };
            if stop {
                return Ok(None);
            }

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
        self.inner.tick().map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Translate a virtual address to a physical address using the current page tables.
    ///
    /// Args:
    ///     vaddr: Virtual address to translate.
    ///
    /// Returns:
    ///     Physical address as ``int``, or raises ``ValueError`` on page fault.
    fn translate(&mut self, vaddr: u64) -> PyResult<u64> {
        use rvsim_core::common::{AccessType, VirtAddr};
        use rvsim_core::sim::state::memory::TranslateResult;
        // The Python binding can't park on a TLB miss, so we walk the PTW
        // synchronously here — emit each PTE MemReq, drain it inline, and
        // continue. This is an FFI-boundary helper; pipeline stages never
        // take this path.
        let mut outcome =
            self.inner.state.core_ctx(0).translate(VirtAddr::new(vaddr), AccessType::Read, 8);
        loop {
            match outcome {
                TranslateResult::Ready(result) => {
                    if let Some(trap) = result.trap {
                        return Err(pyo3::exceptions::PyValueError::new_err(format!(
                            "translation failed for VA {vaddr:#x}: {trap:?}"
                        )));
                    }
                    return Ok(result.paddr.val());
                }
                TranslateResult::NeedPte { pte_addr, state } => {
                    let raw_pte = self.inner.probe_mem_load(pte_addr, 8);
                    outcome = self.inner.state.core_ctx(0).translate_continue(state, raw_pte, 0);
                }
            }
        }
    }

    /// Read raw bytes from physical memory.
    ///
    /// Args:
    ///     paddr: Physical address to read from.
    ///     length: Number of bytes to read.
    ///
    /// Returns:
    ///     ``bytes`` object with the raw memory contents.
    #[pyo3(signature = (paddr, length))]
    fn read_phys_bytes<'py>(
        &mut self,
        py: Python<'py>,
        paddr: u64,
        length: usize,
    ) -> Bound<'py, pyo3::types::PyBytes> {
        if let Some(r) =
            self.inner.state.bus.ram_region().filter(|r| r.contains(paddr, length as u64))
        {
            // SAFETY: bounds-checked by `RamRegion::contains(paddr, length)` above.
            let slice = unsafe { std::slice::from_raw_parts(r.ptr(paddr), length) };
            pyo3::types::PyBytes::new(py, slice)
        } else {
            let mut buf = vec![0u8; length];
            for (i, byte) in buf.iter_mut().enumerate() {
                *byte = self
                    .inner
                    .probe_mem_load(rvsim_core::common::PhysAddr::new(paddr + i as u64), 1)
                    as u8;
            }
            pyo3::types::PyBytes::new(py, &buf)
        }
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
        let width = self.inner.state.config.pipeline.width;
        PyPipelineSnapshot::new(self.inner.pipelines[0].snapshot(width))
    }

    /// Save a checkpoint of the full simulation state to a file.
    ///
    /// The checkpoint includes PC, registers, CSRs, privilege mode, RAM and
    /// the devices' registers.
    fn save(&mut self, path: &str) -> PyResult<()> {
        self.inner.drain();
        let cpu = &self.inner.state;
        let file = std::fs::File::create(path)
            .map_err(|e| PyRuntimeError::new_err(format!("cannot create checkpoint file: {e}")))?;
        let mut w = BufWriter::new(file);

        let mut header = serde_json::Map::new();
        let _ = header.insert("magic".into(), serde_json::Value::from("rvsim-checkpoint"));
        let _ = header.insert("version".into(), serde_json::Value::from(3u64));
        let _ = header.insert("cycle".into(), serde_json::Value::from(cpu.cycle));
        let _ = header.insert("direct_mode".into(), serde_json::Value::from(cpu.direct_mode));
        let _ = header.insert("trace".into(), serde_json::Value::from(cpu.trace.armed));
        let region = cpu.bus.ram_region();
        let ram_start = region.map_or(0, |r| r.base());
        let ram_end = region.map_or(0, |r| r.base() + r.size());
        let _ = header.insert("ram_start".into(), serde_json::Value::from(ram_start));
        let _ = header.insert("ram_end".into(), serde_json::Value::from(ram_end));
        let harts: Vec<serde_json::Value> = cpu.harts.iter().map(hart_to_json).collect();
        let _ = header.insert("harts".into(), serde_json::Value::Array(harts));
        let _ = header.insert("devices".into(), cpu.bus.checkpoint_devices());

        let header_bytes = serde_json::to_vec(&serde_json::Value::Object(header))
            .map_err(|e| PyRuntimeError::new_err(format!("serialization error: {e}")))?;
        let header_len = header_bytes.len() as u64;
        std::io::Write::write_all(&mut w, &header_len.to_le_bytes())
            .map_err(|e| PyRuntimeError::new_err(format!("write error: {e}")))?;
        std::io::Write::write_all(&mut w, &header_bytes)
            .map_err(|e| PyRuntimeError::new_err(format!("write error: {e}")))?;

        if let Some(r) = region {
            let ram_size = r.size() as usize;
            if ram_size > 0 {
                // SAFETY: `r.as_ptr()` is the start of a contiguous DRAM region of
                // exactly `r.size()` bytes owned by the Memory device on the bus.
                let ram_slice = unsafe { std::slice::from_raw_parts(r.as_ptr(), ram_size) };
                std::io::Write::write_all(&mut w, ram_slice)
                    .map_err(|e| PyRuntimeError::new_err(format!("write error: {e}")))?;
            }
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

        self.inner.drain();
        let cpu = &mut self.inner.state;

        cpu.direct_mode = header["direct_mode"].as_bool().unwrap_or(false);
        cpu.trace.armed = header["trace"].as_bool().unwrap_or(false);
        if let Some(cycle) = header["cycle"].as_u64() {
            cpu.cycle = cycle;
        }
        let saved_harts =
            header["harts"].as_array().cloned().unwrap_or_else(|| vec![header.clone()]);
        if saved_harts.len() != cpu.harts.len() {
            return Err(PyRuntimeError::new_err(format!(
                "hart count mismatch: checkpoint has {} harts, simulator has {}",
                saved_harts.len(),
                cpu.harts.len()
            )));
        }
        for (hart, saved) in cpu.harts.iter_mut().zip(&saved_harts) {
            hart_from_json(hart, saved);
        }
        if let Some(devices) = header.get("devices") {
            cpu.bus.restore_devices(devices);
        }

        let ckpt_ram_start = header["ram_start"].as_u64().unwrap_or(0);
        let ckpt_ram_end = header["ram_end"].as_u64().unwrap_or(0);
        let ckpt_ram_size = (ckpt_ram_end - ckpt_ram_start) as usize;
        let region = cpu.bus.ram_region();
        let cpu_ram_size = region.map_or(0, |reg| reg.size() as usize);

        if ckpt_ram_size != cpu_ram_size {
            return Err(PyRuntimeError::new_err(format!(
                "RAM size mismatch: checkpoint has {ckpt_ram_size} bytes, CPU has {cpu_ram_size} bytes"
            )));
        }

        if let Some(reg) = region
            && ckpt_ram_size > 0
        {
            // SAFETY: `reg.as_ptr()` is the start of a contiguous DRAM region of
            // exactly `reg.size()` bytes owned by the Memory device on the bus.
            let ram_slice = unsafe { std::slice::from_raw_parts_mut(reg.as_ptr(), ckpt_ram_size) };
            Read::read_exact(&mut r, ram_slice)
                .map_err(|e| PyRuntimeError::new_err(format!("read error restoring RAM: {e}")))?;
        }

        for core in &mut cpu.cores {
            let _ = core.l1_i_cache.flush();
            let _ = core.l1_d_cache.flush();
            let _ = core.l2_cache.flush();
            core.mmu.dtlb.flush();
            core.mmu.itlb.flush();
            core.mmu.l2_tlb.flush();
        }
        let _ = cpu.l3_cache.flush();
        self.inner.sync_arch_regs();

        Ok(())
    }
}

/// One hart's architectural state as checkpoint JSON.
fn hart_to_json(hart: &rvsim_core::core::Hart) -> serde_json::Value {
    let mut h = serde_json::Map::new();
    let _ = h.insert("pc".into(), serde_json::Value::from(hart.pc));
    let _ = h.insert("privilege".into(), serde_json::Value::from(hart.privilege.to_u8()));
    let _ = h.insert("wfi_waiting".into(), serde_json::Value::from(hart.wfi_waiting));
    let _ = h.insert("sw_seip".into(), serde_json::Value::from(hart.sw_seip));
    let _ =
        h.insert("instructions_retired".into(), serde_json::Value::from(hart.instructions_retired));
    let gprs: Vec<serde_json::Value> = (0u8..32)
        .map(|i| serde_json::Value::from(hart.regs.read(rvsim_core::common::RegIdx::new(i))))
        .collect();
    let _ = h.insert("gpr".into(), serde_json::Value::Array(gprs));
    let fprs: Vec<serde_json::Value> = (0u8..32)
        .map(|i| serde_json::Value::from(hart.regs.read_f(rvsim_core::common::RegIdx::new(i))))
        .collect();
    let _ = h.insert("fpr".into(), serde_json::Value::Array(fprs));
    let c = &hart.csrs;
    let mut csrs = serde_json::Map::new();
    macro_rules! save_csr {
        ($($field:ident),*) => { $( let _ = csrs.insert(stringify!($field).into(), c.$field.into()); )* };
    }
    save_csr!(
        mstatus,
        misa,
        medeleg,
        mideleg,
        mie,
        mtvec,
        mscratch,
        mepc,
        mcause,
        mtval,
        mip,
        sstatus,
        sie,
        stvec,
        sscratch,
        sepc,
        scause,
        stval,
        sip,
        satp,
        mcycle,
        minstret,
        mcountinhibit,
        stimecmp,
        fflags,
        frm,
        mcounteren,
        scounteren,
        menvcfg
    );
    let _ = h.insert("csrs".into(), serde_json::Value::Object(csrs));
    serde_json::Value::Object(h)
}

/// Restores one hart's architectural state from checkpoint JSON.
fn hart_from_json(hart: &mut rvsim_core::core::Hart, saved: &serde_json::Value) {
    hart.pc = saved["pc"].as_u64().unwrap_or(0);
    hart.privilege = PrivilegeMode::from_u8(saved["privilege"].as_u64().unwrap_or(3) as u8);
    hart.wfi_waiting = saved["wfi_waiting"].as_bool().unwrap_or(false);
    hart.sw_seip = saved["sw_seip"].as_bool().unwrap_or(false);
    hart.instructions_retired = saved["instructions_retired"].as_u64().unwrap_or(0);
    if let Some(gprs) = saved["gpr"].as_array() {
        for (i, v) in gprs.iter().enumerate().take(32) {
            hart.regs.write(rvsim_core::common::RegIdx::new(i as u8), v.as_u64().unwrap_or(0));
        }
    }
    if let Some(fprs) = saved["fpr"].as_array() {
        for (i, v) in fprs.iter().enumerate().take(32) {
            hart.regs.write_f(rvsim_core::common::RegIdx::new(i as u8), v.as_u64().unwrap_or(0));
        }
    }
    let Some(csrs) = saved.get("csrs") else { return };
    let c = &mut hart.csrs;
    macro_rules! restore_csr {
        ($($field:ident),*) => { $( if let Some(v) = csrs.get(stringify!($field)).and_then(|v| v.as_u64()) { c.$field = v; } )* };
    }
    restore_csr!(
        mstatus,
        misa,
        medeleg,
        mideleg,
        mie,
        mtvec,
        mscratch,
        mepc,
        mcause,
        mtval,
        mip,
        sstatus,
        sie,
        stvec,
        sscratch,
        sepc,
        scause,
        stval,
        sip,
        satp,
        mcycle,
        minstret,
        mcountinhibit,
        stimecmp,
        fflags,
        frm,
        mcounteren,
        scounteren,
        menvcfg
    );
}
