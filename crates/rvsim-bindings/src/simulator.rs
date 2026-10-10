//! CPU Python binding.
//!
//! Exposes the full simulation interface as a single `Cpu` class. All properties
//! and methods that users interact with live here — nothing leaks through a Python
//! wrapper layer.

use crate::conversion::py_dict_to_config;
use crate::instruction::PyInstruction;
use crate::snapshot::PyPipelineSnapshot;
use crate::stats::PyStats;
use crate::views::{Csrs, Harts, Memory, Registers, VirtualMemory, Width};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use rvsim_core::Simulator;
use rvsim_core::common::{HartId, PhysAddr, VirtAddr};
use rvsim_core::isa::privileged::PrivilegeMode;
use rvsim_core::system::loader::KernelBoot;
use std::io::Write;
use std::io::{BufReader, BufWriter};

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

/// A `run_to` PC stop: one address or any of several.
#[derive(FromPyObject)]
enum PcStop {
    One(u64),
    Any(Vec<u64>),
}

/// The running simulator.
#[pyclass(name = "Simulator", subclass, module = "rvsim._core")]
#[derive(Debug)]
pub struct PySimulator {
    /// The simulator the Python object wraps.
    pub(crate) inner: Simulator,
}

impl PySimulator {
    pub(crate) fn privilege_str(&self, hart: usize) -> &'static str {
        match self.inner.hart(hart).privilege {
            PrivilegeMode::Machine => "M",
            PrivilegeMode::Supervisor => "S",
            PrivilegeMode::User => "U",
        }
    }

    /// Runs for up to `limit` cycles, or until the workload exits, checking
    /// Python signals as it goes.
    fn run_inner(&mut self, py: Python<'_>, limit: Option<u64>) -> PyResult<Option<u64>> {
        use rvsim_core::system::simulator::{StopAt, StopReason};
        let stop = StopAt { cycles: limit, ..StopAt::default() };
        let mut interrupted = None;
        let reason = self
            .inner
            .run_to_with(&stop, || {
                let _ = std::io::stdout().flush();
                match py.check_signals() {
                    Ok(()) => true,
                    Err(error) => {
                        interrupted = Some(error);
                        false
                    }
                }
            })
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let _ = std::io::stdout().flush();
        match reason {
            StopReason::Exited(code) => Ok(Some(code)),
            StopReason::Cycles => Ok(None),
            StopReason::Cancelled => {
                Err(interrupted.unwrap_or_else(|| PyRuntimeError::new_err("run cancelled")))
            }
            other => Err(PyRuntimeError::new_err(format!("run stopped unexpectedly: {other:?}"))),
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
                fmt_commas(self.inner.cycle()),
                fmt_commas(self.inner.instructions_retired()),
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
    ///     config_dict: The nested config dict (from ``Config.to_dict()``).
    ///     elf_data: Raw bytes of an ELF binary (bare-metal mode). Optional.
    ///     kernel_path: Path to a kernel image (kernel mode). Optional.
    ///     firmware_path: Path to an `OpenSBI` ``fw_jump`` image (kernel
    ///         mode). Optional; found under ``software/linux/output`` if absent.
    ///     dtb_path: Path to a DTB file (kernel mode). Optional.
    ///     disk_path: Path to a disk image. Optional.
    #[allow(
        clippy::doc_markdown,
        reason = "Google-style Args: names are bare for the docs generator"
    )]
    #[new]
    #[pyo3(signature = (config_dict, *, elf_data=None, kernel_path=None, firmware_path=None, dtb_path=None, disk_path=None))]
    fn new(
        py: Python<'_>,
        config_dict: &Bound<'_, PyAny>,
        elf_data: Option<Vec<u8>>,
        kernel_path: Option<String>,
        firmware_path: Option<String>,
        dtb_path: Option<String>,
        disk_path: Option<String>,
    ) -> PyResult<Self> {
        let config = py_dict_to_config(py, config_dict)?;
        config.validate().map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        let mut sim = Simulator::build(&config, &disk_path.unwrap_or_default());
        if let Some(data) = elf_data {
            sim.load_elf(&data).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        }
        if let Some(kernel) = kernel_path {
            let boot = KernelBoot { kernel: Some(kernel), firmware: firmware_path, dtb: dtb_path };
            sim.boot_kernel(&boot).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        }
        Ok(Self { inner: sim })
    }

    /// The architectural PC (the next instruction to retire). Writing it
    /// drops everything in flight and restarts fetch there.
    #[getter]
    fn pc(&self) -> u64 {
        self.inner.hart(0).pc
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
    /// to; `trace_filter` narrows them further.
    #[getter]
    const fn trace(&self) -> bool {
        self.inner.trace().armed
    }

    #[setter]
    const fn set_trace(&mut self, value: bool) {
        self.inner.trace_mut().armed = value;
    }

    /// Narrows what an armed trace prints.
    ///
    /// Args:
    ///     harts: Hart ids whose events print; ``None`` for every hart.
    ///     cycles: ``(first, last)`` cycle window; ``None`` for no window.
    ///     trap_causes: `mcause` values (interrupt bit included) whose
    ///         trap-taken events print; ``None`` prints every trap except
    ///         timer interrupts and ecalls.
    #[allow(
        clippy::doc_markdown,
        reason = "Google-style Args: names are bare for the docs generator"
    )]
    #[pyo3(signature = (harts=None, cycles=None, trap_causes=None))]
    fn trace_filter(
        &mut self,
        harts: Option<Vec<u32>>,
        cycles: Option<(u64, u64)>,
        trap_causes: Option<Vec<u64>>,
    ) {
        let trace = self.inner.trace_mut();
        trace.harts = harts.unwrap_or_default().into_iter().map(HartId::new).collect();
        trace.cycle_from = cycles.map(|(from, _)| from);
        trace.cycle_to = cycles.map(|(_, to)| to);
        trace.trap_causes = trap_causes.unwrap_or_default();
    }

    /// A snapshot of the stats since the start of simulation, the last
    /// checkpoint restore or the last `reset_stats`.
    ///
    /// Subtracting two snapshots gives the stats of the region between them
    /// while the whole run's stats stay intact: counters are subtracted and
    /// derived stats (IPC, miss rates, accuracies) recomputed from the
    /// differences. Histograms keep exact counts, sums and means but report
    /// no minimum or maximum, which two cumulative snapshots cannot recover.
    /// Subtracting a later snapshot, or across a `reset_stats`,
    /// raises ``ValueError``.
    ///
    /// Example:
    ///     ```python
    ///     start = cpu.stats
    ///     cpu.run(limit=1_000_000)
    ///     region = cpu.stats - start
    ///     print(region.ipc, region["core0.bp.committed.accuracy"])
    ///     ```
    #[getter]
    fn stats(&self) -> PyStats {
        let (cycles, instructions_retired) = self.inner.stats_window();
        let epoch = self.inner.stats_epoch();
        PyStats::new(self.inner.stats().clone(), cycles, instructions_retired, epoch)
    }

    /// Whether idle cores (waiting in WFI with nothing in flight) have their
    /// cycles counted instead of ticked, and cycles in which the whole
    /// system only waits for a timer or device are skipped. On by default;
    /// results are the same either way, so this exists to check that.
    #[getter]
    const fn skip_idle_cores(&self) -> bool {
        self.inner.skip_idle_cores()
    }

    #[setter]
    const fn set_skip_idle_cores(&mut self, skip: bool) {
        self.inner.set_skip_idle_cores(skip);
    }

    /// Whether every cache invariant (each cache's bookkeeping, inclusion,
    /// the copies each level records above it, coherence) is checked after
    /// every event. Off by default; with it on a run is many times slower,
    /// and the first broken invariant ends it with an error naming it.
    #[getter]
    const fn audit_caches(&self) -> bool {
        self.inner.audit_caches()
    }

    #[setter]
    const fn set_audit_caches(&mut self, audit: bool) {
        self.inner.set_audit_caches(audit);
    }

    /// Zero every stat; `stats` then counts from here, as gem5's
    /// ``m5 resetstats`` does. Prefer subtracting snapshots, which keeps the
    /// whole run's stats.
    fn reset_stats(&mut self) {
        self.inner.reset_stats();
    }

    /// The stats the guest dumped through the sim-control device, oldest
    /// first, as ``(label, Stats)`` pairs; each covers the window since the
    /// last reset before it. Software running in the guest marks its own
    /// regions this way; `stats_between` subtracts two dumps.
    #[getter]
    fn stats_dumps(&self) -> Vec<(u64, PyStats)> {
        self.inner
            .stats_dumps()
            .iter()
            .map(|dump| {
                let stats = PyStats::new(
                    dump.stats.clone(),
                    dump.cycles,
                    dump.instructions_retired,
                    dump.epoch,
                );
                (dump.label, stats)
            })
            .collect()
    }

    /// The stats of the region between the guest's dump labelled `start`
    /// and the next dump after it labelled `end`, with the whole run's
    /// stats left intact.
    ///
    /// Raises ``KeyError`` when no such pair was dumped and ``ValueError``
    /// when the stats were reset between them.
    fn stats_between(&self, start: u64, end: u64) -> PyResult<PyStats> {
        let dumps = self.stats_dumps();
        let missing =
            || pyo3::exceptions::PyKeyError::new_err(format!("no dumps {start} then {end}"));
        let first = dumps.iter().position(|(label, _)| *label == start).ok_or_else(missing)?;
        let (_, later) =
            dumps[first + 1..].iter().find(|(label, _)| *label == end).ok_or_else(missing)?;
        later.since(&dumps[first].1)
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

    /// Cycles since the system started, carried across checkpoints.
    #[getter]
    const fn cycle(&self) -> u64 {
        self.inner.cycle()
    }

    /// Instructions retired by every hart since the system started,
    /// carried across checkpoints.
    #[getter]
    fn instructions_retired(&self) -> u64 {
        self.inner.instructions_retired()
    }

    /// Number of harts in the system.
    #[getter]
    const fn hart_count(&self) -> usize {
        self.inner.hart_count()
    }

    /// Memory view for 32-bit reads — ``cpu.mem32[addr]``.
    #[getter]
    fn mem32(slf: Bound<'_, Self>) -> Memory {
        Memory { cpu: slf.unbind(), width: Width::Bits32 }
    }

    /// Memory view for 64-bit reads — ``cpu.mem64[addr]``.
    #[getter]
    fn mem64(slf: Bound<'_, Self>) -> Memory {
        Memory { cpu: slf.unbind(), width: Width::Bits64 }
    }

    /// Virtual memory view for 32-bit reads — ``cpu.vmem32[vaddr]``.
    ///
    /// Translates the virtual address through the current page tables (SATP)
    /// before reading. Raises ``ValueError`` if translation fails.
    #[getter]
    fn vmem32(slf: Bound<'_, Self>) -> VirtualMemory {
        VirtualMemory { cpu: slf.unbind(), width: Width::Bits32 }
    }

    /// Virtual memory view for 64-bit reads — ``cpu.vmem64[vaddr]``.
    ///
    /// Translates the virtual address through the current page tables (SATP)
    /// before reading. Raises ``ValueError`` if translation fails.
    #[getter]
    fn vmem64(slf: Bound<'_, Self>) -> VirtualMemory {
        VirtualMemory { cpu: slf.unbind(), width: Width::Bits64 }
    }

    /// Committed PC trace from the pipeline as a list of ``(pc, raw_inst)`` pairs.
    #[getter]
    fn pc_trace(&self) -> Vec<(u64, u32)> {
        self.inner.pc_trace(0).to_vec()
    }

    /// Open a commit log file. Each retired instruction is a line
    /// ``core   0: 0x<pc> (0x<inst>) priv <mode> cycle <n>`` (``<n>`` the
    /// cycle it retired in) followed by its effects: ``x<n>``/``f<n>
    /// 0x<value>``, ``c<csr> 0x<value>``, and ``load``/``store 0x<vaddr>
    /// 0x<paddr> <bytes> 0x<value>``. Each trap taken is a line ``core   0:
    /// trap 0x<cause> 0x<epc> 0x<tval> cycle <n>``. Requires the
    /// ``commit-log`` feature.
    #[cfg(feature = "commit-log")]
    fn open_commit_log(&mut self, path: &str) -> PyResult<()> {
        self.inner.open_commit_log(path).map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Execute until one instruction commits.
    ///
    /// Returns an `Instruction` or ``None`` if the simulation exited
    /// before an instruction could commit.
    #[pyo3(signature = (max_cycles=100_000))]
    fn step(&mut self, py: Python<'_>, max_cycles: u64) -> PyResult<Option<PyInstruction>> {
        let before_last = self.inner.pc_trace(0).last().copied();
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

            let new_last = self.inner.pc_trace(0).last().copied();
            if new_last != before_last
                && let Some((pc, inst)) = new_last
            {
                let asm = rvsim_core::isa::disasm::disassemble(inst);
                return Ok(Some(PyInstruction { pc, raw: inst, asm, cycles: self.inner.cycle() }));
            }
        }
    }

    /// Run the simulation until exit or cycle limit.
    ///
    /// Args:
    ///     limit: Max cycles to simulate. ``None`` means unlimited.
    ///     progress: Print progress to stderr every N cycles. 0 = silent.
    ///     stats_sections: Print stats on completion. ``None`` suppresses
    ///         the report; ``[]`` prints all subjects; a list of subjects
    ///         (e.g. ``["core0", "hart0"]``) restricts output to those
    ///         subjects. Use `Stats.subjects` to enumerate.
    ///
    /// Returns:
    ///     Exit code or ``None`` if *limit* was reached without exiting.
    #[allow(
        clippy::doc_markdown,
        reason = "Google-style Args: names are bare for the docs generator"
    )]
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
            let (cycle, retired) = (self.inner.cycle(), self.inner.instructions_retired());
            let text = if sections.is_empty() {
                self.inner.stats().summary(cycle, retired)
            } else {
                let refs: Vec<&str> = sections.iter().map(String::as_str).collect();
                self.inner.stats().summary_sections(cycle, retired, &refs)
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
    ///     List of `Stats` snapshots, one per interval.
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

            let (cycles, instructions_retired) = self.inner.stats_window();
            let epoch = self.inner.stats_epoch();
            snapshots.push(PyStats::new(
                self.inner.stats().clone(),
                cycles,
                instructions_retired,
                epoch,
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
        predicate: Option<&Bound<'_, PyAny>>,
        pc: Option<u64>,
        privilege: Option<&str>,
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
                pc.is_some_and(|p| cpu.inner.hart(0).pc == p)
                    || privilege.is_some_and(|priv_str| cpu.privilege_str(0) == priv_str)
            };
            if stop {
                return Ok(None);
            }

            if let Some(pred) = predicate {
                let result = pred.call1((slf_py.clone_ref(py),))?;
                if result.extract::<bool>()? {
                    return Ok(None);
                }
            }

            py.check_signals()?;
        }
    }

    /// Run until a condition holds, checking after every cycle, and say
    /// which: ``("exit", code)``, ``("cycles", None)``,
    /// ``("instructions", None)``, ``("pc", hart)``, ``("break", label)`` or
    /// ``("console", None)``.
    ///
    /// Args:
    ///     cycles: Stop after this many cycles.
    ///     instructions: Stop once this many more instructions have retired
    ///         (all harts).
    ///     pc: Stop when any hart's next instruction to retire is at this
    ///         address, or at any of a list of them.
    ///     guest_breaks: Stop when guest software runs ``rvsim break``.
    ///     console_output: Stop when a captured console holds output
    ///         `read_console` has not taken.
    ///
    /// Runs at least one cycle, so running on from a stop at ``pc`` moves
    /// past it.
    #[allow(
        clippy::doc_markdown,
        reason = "Google-style Args: names are bare for the docs generator"
    )]
    #[pyo3(signature = (*, cycles=None, instructions=None, pc=None, guest_breaks=true, console_output=false))]
    fn run_to(
        &mut self,
        py: Python<'_>,
        cycles: Option<u64>,
        instructions: Option<u64>,
        pc: Option<PcStop>,
        guest_breaks: bool,
        console_output: bool,
    ) -> PyResult<(String, Option<u64>)> {
        use rvsim_core::system::simulator::{StopAt, StopReason};
        let pcs = match pc {
            None => Vec::new(),
            Some(PcStop::One(pc)) => vec![pc],
            Some(PcStop::Any(pcs)) => pcs,
        };
        let stop = StopAt { cycles, instructions, pcs, guest_breaks, console_output };
        let mut interrupted = None;
        let reason = self
            .inner
            .run_to_with(&stop, || match py.check_signals() {
                Ok(()) => true,
                Err(error) => {
                    interrupted = Some(error);
                    false
                }
            })
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
        let _ = std::io::stdout().flush();
        Ok(match reason {
            StopReason::Exited(code) => ("exit".into(), Some(code)),
            StopReason::Cycles => ("cycles".into(), None),
            StopReason::Instructions => ("instructions".into(), None),
            StopReason::Pc { hart } => ("pc".into(), Some(hart as u64)),
            StopReason::GuestBreak { label } => ("break".into(), Some(label)),
            StopReason::ConsoleOutput => ("console".into(), None),
            StopReason::Cancelled => {
                return Err(interrupted.unwrap_or_else(|| PyRuntimeError::new_err("run cancelled")));
            }
        })
    }

    /// The console output since the last call, when the config's
    /// ``console`` is ``"captured"``.
    fn read_console(&mut self) -> String {
        String::from_utf8_lossy(&self.inner.take_console_output()).into_owned()
    }

    /// Type ``text`` into a captured console.
    fn write_console(&mut self, text: &str) {
        self.inner.send_console_input(text.as_bytes());
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
        self.inner.translate_now(0, VirtAddr::new(vaddr)).map(|paddr| paddr.val()).map_err(|trap| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "translation failed for VA {vaddr:#x}: {trap:?}"
            ))
        })
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
        if let Some(bytes) = self.inner.read_phys_bytes(PhysAddr::new(paddr), length) {
            pyo3::types::PyBytes::new(py, &bytes)
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
    /// Returns a `PipelineSnapshot` with the contents of every inter-stage
    /// latch as of the *end* of the last ``tick()``.  Call after ``tick()`` or
    /// ``step()`` to inspect what is currently in-flight.
    ///
    /// This performs a shallow clone of the latch vectors — it has no effect on
    /// simulation correctness or timing.
    fn pipeline_snapshot(&self) -> PyPipelineSnapshot {
        PyPipelineSnapshot::new(self.inner.pipeline_snapshot(0))
    }

    /// Save a checkpoint of the system's architectural state to ``path``.
    ///
    /// A checkpoint holds RAM (skipping 4 KiB pages of zeros), the cycle
    /// counter, every hart's architectural state (PC, privilege, integer,
    /// floating-point and vector registers, every CSR, PMP entries and its
    /// LR reservation) and the devices' registers (CLINT timers and
    /// ``mtime``, PLIC priorities, enables, thresholds and claims, UART
    /// registers and unread input, the virtio disk's queue and every sector
    /// the guest has written).
    ///
    /// Saving first drains the machine the way gem5 does: speculative work
    /// is discarded, committed stores still in the store buffers reach RAM,
    /// each hart is left at its committed PC and a disk request in flight
    /// completes at once, so a run that continues after a save is not
    /// cycle-identical to one without it.
    fn save(&mut self, path: &str) -> PyResult<()> {
        let file = std::fs::File::create(path)
            .map_err(|e| PyRuntimeError::new_err(format!("cannot create checkpoint file: {e}")))?;
        self.inner
            .save_checkpoint(&mut BufWriter::new(file))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }

    /// Restore a checkpoint saved by `save`.
    ///
    /// A checkpoint restores into any configuration with the same hart
    /// count, RAM size and VLEN, so a system can boot on a cheap
    /// configuration and continue on a detailed one. It does not hold cache
    /// contents, TLBs, predictor state or in-flight memory traffic: after a
    /// restore the caches, TLBs and the coherence home agent start empty, as
    /// gem5's do, so warm the system up before measuring. The disk's written
    /// sectors are replayed over the image this simulator loaded, so it must
    /// load the same image the checkpoint was taken on; the image file
    /// itself is never modified. A restore into a mismatched system, or onto
    /// a different disk image, raises an error naming what differs and
    /// leaves the simulator untouched.
    fn restore(&mut self, path: &str) -> PyResult<()> {
        let file = std::fs::File::open(path)
            .map_err(|e| PyRuntimeError::new_err(format!("cannot open checkpoint file: {e}")))?;
        self.inner
            .restore_checkpoint(&mut BufReader::new(file))
            .map_err(|e| PyRuntimeError::new_err(e.to_string()))
    }
}
