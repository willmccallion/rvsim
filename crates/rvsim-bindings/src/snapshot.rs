//! Pipeline snapshot Python binding.
//!
//! Exposes `PipelineSnapshot` as a read-only Python class with built-in
//! `render()` / `visualize()` methods. All stage latches are lists of per-slot
//! dicts. No private methods are exposed.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rvsim_core::common::VirtAddr;
use rvsim_core::disasm::disassemble;
use rvsim_core::system::snapshot::{
    DecodedSlot, ExecutedSlot, FetchSlot, FetchedSlot, MemorySlot, PipelineSnapshot, RenamedSlot,
    WritebackSlot,
};

/// A latch slot as Python sees it: a dict of its fields.
trait SlotFields {
    /// Adds the slot's fields to `dict`.
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()>;
}

/// `slots` as a list of dicts.
fn slot_list<T: SlotFields>(py: Python<'_>, slots: &[T]) -> PyResult<Py<PyList>> {
    let dicts = slots
        .iter()
        .map(|slot| {
            let dict = PyDict::new(py);
            slot.fill(&dict)?;
            Ok(dict)
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(PyList::new(py, dicts)?.unbind())
}

/// The fields every slot past Fetch1 has: its address and instruction.
fn fill_instruction(dict: &Bound<'_, PyDict>, pc: u64, raw: u32) -> PyResult<()> {
    dict.set_item("pc", pc)?;
    dict.set_item("raw", raw)?;
    dict.set_item("asm", disassemble(raw))
}

/// The prediction fields of a slot in fetch.
fn fill_prediction(dict: &Bound<'_, PyDict>, predicted_target: Option<VirtAddr>) -> PyResult<()> {
    dict.set_item("pred_taken", predicted_target.is_some())?;
    dict.set_item("pred_target", predicted_target.map(|target| target.val()))
}

impl SlotFields for FetchSlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        dict.set_item("pc", self.pc.val())?;
        fill_prediction(dict, self.predicted_target)
    }
}

impl SlotFields for FetchedSlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        fill_instruction(dict, self.pc.val(), self.raw)?;
        fill_prediction(dict, self.predicted_target)
    }
}

impl SlotFields for DecodedSlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        fill_instruction(dict, self.pc.val(), self.raw)?;
        dict.set_item("rs1", self.rs1.as_u8())?;
        dict.set_item("rs2", self.rs2.as_u8())?;
        dict.set_item("rd", self.rd.as_u8())?;
        dict.set_item("imm", self.imm)
    }
}

impl SlotFields for RenamedSlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        fill_instruction(dict, self.pc.val(), self.raw)?;
        dict.set_item("rs1", self.rs1.as_u8())?;
        dict.set_item("rs2", self.rs2.as_u8())?;
        dict.set_item("rd", self.rd.as_u8())?;
        dict.set_item("rv1", self.rv1)?;
        dict.set_item("rv2", self.rv2)?;
        dict.set_item("rob_tag", self.rob_tag)?;
        dict.set_item("rs1_ready", self.rs1_ready)?;
        dict.set_item("rs2_ready", self.rs2_ready)
    }
}

impl SlotFields for ExecutedSlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        fill_instruction(dict, self.pc.val(), self.raw)?;
        dict.set_item("rd", self.rd.as_u8())?;
        dict.set_item("alu", self.alu)?;
        dict.set_item("store_data", self.store_data)?;
        dict.set_item("rob_tag", self.rob_tag)
    }
}

impl SlotFields for MemorySlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        fill_instruction(dict, self.pc.val(), self.raw)?;
        dict.set_item("rd", self.rd.as_u8())?;
        dict.set_item("alu", self.alu)?;
        dict.set_item("vaddr", self.vaddr.val())?;
        dict.set_item("paddr", self.paddr.val())?;
        dict.set_item("store_data", self.store_data)?;
        dict.set_item("rob_tag", self.rob_tag)
    }
}

impl SlotFields for WritebackSlot {
    fn fill(&self, dict: &Bound<'_, PyDict>) -> PyResult<()> {
        fill_instruction(dict, self.pc.val(), self.raw)?;
        dict.set_item("rd", self.rd.as_u8())?;
        dict.set_item("alu", self.alu)?;
        dict.set_item("load_data", self.load_data)?;
        dict.set_item("value", self.value)?;
        dict.set_item("rob_tag", self.rob_tag)
    }
}

const COL_W: usize = 14;
const SLOT_W: usize = 4;

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// One cell: the instruction `raw`'s mnemonic and operands, truncated to
/// `COL_W`.
fn asm_cell(raw: u32) -> String {
    trunc(&disassemble(raw), COL_W)
}

/// An issue-queue cell, marked when an operand is not ready.
fn issue_cell(slot: &RenamedSlot) -> String {
    let asm = asm_cell(slot.raw);
    if slot.rs1_ready && slot.rs2_ready { asm } else { trunc(&format!("⋯{asm}"), COL_W) }
}

/// One stage's column of the diagram: its header and a cell per slot.
struct StageColumn {
    header: &'static str,
    cells: Vec<Option<String>>,
}

impl StageColumn {
    /// The column of `slots`, the first `width` of them drawn by `cell`.
    fn of<T>(header: &'static str, slots: &[T], width: usize, cell: impl Fn(&T) -> String) -> Self {
        Self { header, cells: (0..width).map(|i| slots.get(i).map(&cell)).collect() }
    }

    /// A column for a stage with no outbound latch to show.
    fn empty(header: &'static str, width: usize) -> Self {
        Self { header, cells: vec![None; width] }
    }
}

/// Every stage's column, fetch to commit.
fn stage_columns(snap: &PipelineSnapshot) -> [StageColumn; 10] {
    let w = snap.width;
    [
        StageColumn::of("F1", &snap.fetch1_fetch2, w, |e| format!("{:#010x}", e.pc)),
        StageColumn::of("F2", &snap.fetch2_decode, w, |e| asm_cell(e.raw)),
        StageColumn::of("DE", &snap.decode_rename, w, |e| asm_cell(e.raw)),
        StageColumn::of("RN", &snap.rename_issue, w, |e| asm_cell(e.raw)),
        StageColumn::of("IS", &snap.issue_queue, w, issue_cell),
        StageColumn::of("EX", &snap.execute_mem1, w, |e| asm_cell(e.raw)),
        StageColumn::of("M1", &snap.mem1_mem2, w, |e| asm_cell(e.raw)),
        StageColumn::of("M2", &snap.mem2_wb, w, |e| asm_cell(e.raw)),
        StageColumn::empty("WB", w),
        StageColumn::empty("CM", w),
    ]
}

/// The values being forwarded this cycle, as `reg←value`.
fn forwarding_notes(snap: &PipelineSnapshot) -> Vec<String> {
    let executed = snap.execute_mem1.iter().map(|e| (e.rd, e.alu));
    let written = snap.mem2_wb.iter().map(|e| (e.rd, e.value));
    executed
        .chain(written)
        .filter(|(rd, _)| !rd.is_zero())
        .map(|(rd, value)| format!("{}←{value:#x}", rd.int_abi_name()))
        .collect()
}

fn render_inner(snap: &PipelineSnapshot) -> String {
    let columns = stage_columns(snap);
    let headers: Vec<String> = columns.iter().map(|c| format!("{:^COL_W$}", c.header)).collect();
    let header_line = format!("{:SLOT_W$} {}", "", headers.join(" "));
    let rule = "─".repeat(header_line.len());

    let mut out = vec![rule.clone(), header_line, rule.clone()];
    for slot in 0..snap.width {
        let cells: Vec<String> = columns
            .iter()
            .map(|c| format!("{:<COL_W$}", c.cells[slot].as_deref().unwrap_or("─")))
            .collect();
        out.push(format!("[{slot}]  {}", cells.join(" ")));
    }
    let notes = forwarding_notes(snap);
    if !notes.is_empty() {
        out.push(rule.clone());
        out.push(format!("  fwd: {}", notes.join("  ")));
    }
    out.push(rule);
    out.join("\n")
}

/// Point-in-time snapshot of all pipeline inter-stage latches.
///
/// Obtained via ``cpu.pipeline_snapshot()`` after any ``tick()`` or ``step()``.
///
/// Each stage attribute is a list of slot dicts (length ≤ ``width``).
/// An empty list means the stage is idle or stalled this cycle.
///
/// All slots include ``pc``, and every slot past ``fetch1_fetch2``, whose
/// instructions are not read yet, ``raw`` and ``asm``. Later stages add:
///
/// - ``decode_rename``: ``rs1``, ``rs2``, ``rd``, ``imm``
/// - ``issue_queue``: ``rs1``, ``rs2``, ``rd``, ``rv1``, ``rv2``,
///   ``rob_tag``, ``rs1_ready``, ``rs2_ready``
/// - ``execute_mem1`` / ``mem1_mem2``: ``rd``, ``alu``, ``store_data``, ``rob_tag``
/// - ``mem1_mem2``: also ``vaddr``, ``paddr``
/// - ``mem2_wb``: ``rd``, ``alu``, ``load_data``, ``value`` (what it
///   writes to ``rd``), ``rob_tag``
#[pyclass(name = "PipelineSnapshot", subclass, module = "rvsim._core")]
#[derive(Debug)]
pub struct PyPipelineSnapshot {
    inner: PipelineSnapshot,
}

impl PyPipelineSnapshot {
    /// Wraps a snapshot for Python.
    #[must_use]
    pub const fn new(inner: PipelineSnapshot) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyPipelineSnapshot {
    /// Pipeline width (superscalar degree).
    #[getter]
    const fn width(&self) -> usize {
        self.inner.width
    }

    /// Fetch1 → Fetch2 latch, holding addresses whose instructions are
    /// not read yet.
    ///
    /// Each slot: ``{pc, pred_taken, pred_target}``; ``pred_target`` is
    /// ``None`` unless predicted taken.
    #[getter]
    fn fetch1_fetch2(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.fetch1_fetch2)
    }

    /// Fetch2 → Decode latch.
    ///
    /// Each slot: ``{pc, raw, asm, pred_taken, pred_target}``;
    /// ``pred_target`` is ``None`` unless predicted taken.
    #[getter]
    fn fetch2_decode(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.fetch2_decode)
    }

    /// Decode → Rename latch.
    ///
    /// Each slot: ``{pc, raw, asm, rs1, rs2, rd, imm}``; operands are read
    /// at issue, not decode.
    #[getter]
    fn decode_rename(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.decode_rename)
    }

    /// Rename → Issue latch (ROB-allocated instructions pending dispatch).
    ///
    /// Each slot: ``{pc, raw, asm, rs1, rs2, rd, rv1, rv2, rob_tag, rs1_ready, rs2_ready}``.
    #[getter]
    fn rename_issue(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.rename_issue)
    }

    /// Issue queue (Rename → Execute, waiting for operands).
    ///
    /// Listed front-to-back (oldest first). Each slot:
    /// ``{pc, raw, asm, rs1, rs2, rd, rv1, rv2, rob_tag, rs1_ready, rs2_ready}``.
    #[getter]
    fn issue_queue(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.issue_queue)
    }

    /// Execute → Memory1 latch.
    ///
    /// Each slot: ``{pc, raw, asm, rd, alu, store_data, rob_tag}``.
    ///
    /// ``alu`` is the forwarded result available to dependent instructions.
    #[getter]
    fn execute_mem1(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.execute_mem1)
    }

    /// Memory1 → Memory2 latch.
    ///
    /// Each slot: ``{pc, raw, asm, rd, alu, vaddr, paddr, store_data, rob_tag}``.
    #[getter]
    fn mem1_mem2(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.mem1_mem2)
    }

    /// Memory2 → Writeback latch.
    ///
    /// Each slot: ``{pc, raw, asm, rd, alu, load_data, rob_tag}``.
    ///
    /// ``load_data`` carries the forwarded value for load instructions.
    #[getter]
    fn mem2_wb(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        slot_list(py, &self.inner.mem2_wb)
    }

    /// Return the pipeline diagram as a string.
    fn render(&self) -> String {
        render_inner(&self.inner)
    }

    /// Print the pipeline diagram to stdout.
    fn visualize(&self) {
        println!("{}", render_inner(&self.inner));
    }

    fn __repr__(&self) -> String {
        let counts = [
            ("fetch1_fetch2", self.inner.fetch1_fetch2.len()),
            ("fetch2_decode", self.inner.fetch2_decode.len()),
            ("decode_rename", self.inner.decode_rename.len()),
            ("rename_issue", self.inner.rename_issue.len()),
            ("issue_queue", self.inner.issue_queue.len()),
            ("execute_mem1", self.inner.execute_mem1.len()),
            ("mem1_mem2", self.inner.mem1_mem2.len()),
            ("mem2_wb", self.inner.mem2_wb.len()),
        ];
        let summary: Vec<String> =
            counts.iter().filter(|(_, n)| *n > 0).map(|(name, n)| format!("{name}={n}")).collect();
        format!(
            "PipelineSnapshot(width={}, {})",
            self.inner.width,
            if summary.is_empty() { "idle".to_string() } else { summary.join(", ") }
        )
    }
}
