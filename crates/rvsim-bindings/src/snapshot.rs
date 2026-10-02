//! Pipeline snapshot Python binding.
//!
//! Exposes `PipelineSnapshot` as a read-only Python class with built-in
//! `render()` / `visualize()` methods. All stage latches are lists of per-slot
//! dicts. No private methods are exposed.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rvsim_core::isa::disasm::disassemble;
use rvsim_core::isa::reg::RegIdx;
use rvsim_core::uarch::pipeline::snapshot::PipelineSnapshot;

const ABI: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

fn reg_name(idx: RegIdx) -> &'static str {
    ABI.get(idx.as_usize()).copied().unwrap_or("x?")
}

fn slot_dict(py: Python<'_>, pc: u64, raw: u32) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("pc", pc)?;
    d.set_item("raw", raw)?;
    d.set_item("asm", disassemble(raw))?;
    Ok(d)
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

/// One cell: mnemonic + first operand, truncated to `COL_W`.
fn cell(asm: &str) -> String {
    trunc(asm, COL_W)
}

fn render_inner(snap: &PipelineSnapshot) -> String {
    let w = snap.width;

    #[allow(clippy::items_after_statements)]
    struct StageCol {
        hdr: &'static str,
        cells: Vec<Option<String>>,
    }

    let mut cols: Vec<StageCol> = Vec::new();

    macro_rules! stage {
        ($hdr:expr, $entries:expr, $cell_fn:expr) => {{
            let mut cells: Vec<Option<String>> = vec![None; w];
            for (i, e) in $entries.iter().enumerate() {
                if i < w {
                    cells[i] = Some($cell_fn(e));
                }
            }
            cols.push(StageCol { hdr: $hdr, cells });
        }};
    }

    stage!(
        "F1",
        snap.fetch1_fetch2,
        |e: &rvsim_core::uarch::pipeline::latches::Fetch1Fetch2Entry| { format!("{:#010x}", e.pc) }
    );

    stage!("F2", snap.fetch2_decode, |e: &rvsim_core::uarch::pipeline::latches::IfIdEntry| {
        cell(&disassemble(e.inst))
    });

    stage!("DE", snap.decode_rename, |e: &rvsim_core::uarch::pipeline::latches::IdExEntry| {
        cell(&disassemble(e.inst.bits))
    });

    stage!(
        "RN",
        snap.rename_issue,
        |e: &rvsim_core::uarch::pipeline::latches::RenameIssueEntry| {
            cell(&disassemble(e.inst.bits))
        }
    );

    stage!("IS", snap.issue_queue, |e: &rvsim_core::uarch::pipeline::latches::RenameIssueEntry| {
        let asm = disassemble(e.inst.bits);
        let stalled = e.rs1_tag.is_some() || e.rs2_tag.is_some();
        if stalled { trunc(&format!("⋯{}", cell(&asm)), COL_W) } else { cell(&asm) }
    });

    stage!("EX", snap.execute_mem1, |e: &rvsim_core::uarch::pipeline::latches::ExMem1Entry| {
        cell(&disassemble(e.inst))
    });

    stage!("M1", snap.mem1_mem2, |e: &rvsim_core::uarch::pipeline::latches::Mem1Mem2Entry| {
        cell(&disassemble(e.inst))
    });

    stage!("M2", snap.mem2_wb, |e: &rvsim_core::uarch::pipeline::latches::Mem2WbEntry| {
        cell(&disassemble(e.inst))
    });

    // WB and CM have no outbound latch to inspect — both show empty.
    {
        let cells = vec![None; w];
        cols.push(StageCol { hdr: "WB", cells });
    }
    {
        let cells = vec![None; w];
        cols.push(StageCol { hdr: "CM", cells });
    }

    let hdr_cells: Vec<String> = cols.iter().map(|c| format!("{:^COL_W$}", c.hdr)).collect();
    let hdr_line = format!("{:SLOT_W$} {}", "", hdr_cells.join(" "));
    let rule = "─".repeat(hdr_line.len());

    let mut rows: Vec<String> = Vec::new();
    for slot in 0..w {
        let row_cells: Vec<String> = cols
            .iter()
            .map(|c| {
                let s = c.cells[slot]
                    .as_ref()
                    .map_or_else(|| "─".to_string(), std::clone::Clone::clone);
                format!("{s:<COL_W$}")
            })
            .collect();
        rows.push(format!("[{slot}]  {}", row_cells.join(" ")));
    }

    let mut notes: Vec<String> = Vec::new();
    for e in &snap.execute_mem1 {
        if !e.rd.is_zero() {
            notes.push(format!("{}←{:#x}", reg_name(e.rd), e.alu));
        }
    }
    for e in &snap.mem2_wb {
        if !e.rd.is_zero() {
            let v = if e.load_data != 0 { e.load_data } else { e.alu };
            notes.push(format!("{}←{:#x}", reg_name(e.rd), v));
        }
    }

    let mut out: Vec<String> = Vec::new();
    out.push(rule.clone());
    out.push(hdr_line);
    out.push(rule.clone());
    out.extend(rows);
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
/// All slots include ``pc``, ``raw``, ``asm``. Later stages add:
///
/// - ``decode_rename``: ``rs1``, ``rs2``, ``rd``, ``imm``, ``rv1``, ``rv2``
/// - ``issue_queue``: ``rs1``, ``rs2``, ``rd``, ``rv1``, ``rv2``,
///   ``rob_tag``, ``rs1_ready``, ``rs2_ready``
/// - ``execute_mem1`` / ``mem1_mem2``: ``rd``, ``alu``, ``store_data``, ``rob_tag``
/// - ``mem1_mem2``: also ``vaddr``, ``paddr``
/// - ``mem2_wb``: ``rd``, ``alu``, ``load_data``, ``rob_tag``
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

    /// Fetch1 → Fetch2 latch.
    ///
    /// Each slot: ``{pc, raw, asm, pred_taken, pred_target}``.
    #[getter]
    fn fetch1_fetch2(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .fetch1_fetch2
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.pc, 0)?;
                d.set_item("pred_taken", e.pred_taken)?;
                d.set_item("pred_target", e.pred_target)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Fetch2 → Decode latch.
    ///
    /// Each slot: ``{pc, raw, asm, pred_taken, pred_target}``.
    #[getter]
    fn fetch2_decode(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .fetch2_decode
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("pred_taken", e.pred_taken)?;
                d.set_item("pred_target", e.pred_target)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Decode → Rename latch.
    ///
    /// Each slot: ``{pc, raw, asm, rs1, rs2, rd, imm, rv1, rv2}``.
    #[getter]
    fn decode_rename(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .decode_rename
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.inst.pc, e.inst.bits)?;
                d.set_item("rs1", e.inst.rs1.as_u8())?;
                d.set_item("rs2", e.inst.rs2.as_u8())?;
                d.set_item("rd", e.inst.rd.as_u8())?;
                d.set_item("imm", e.inst.imm)?;
                d.set_item("rv1", e.inst.rv1)?;
                d.set_item("rv2", e.inst.rv2)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Rename → Issue latch (ROB-allocated instructions pending dispatch).
    ///
    /// Each slot: ``{pc, raw, asm, rs1, rs2, rd, rv1, rv2, rob_tag, rs1_ready, rs2_ready}``.
    #[getter]
    fn rename_issue(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .rename_issue
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.inst.pc, e.inst.bits)?;
                d.set_item("rs1", e.inst.rs1.as_u8())?;
                d.set_item("rs2", e.inst.rs2.as_u8())?;
                d.set_item("rd", e.inst.rd.as_u8())?;
                d.set_item("rv1", e.inst.rv1)?;
                d.set_item("rv2", e.inst.rv2)?;
                d.set_item("rob_tag", e.rob_tag.0)?;
                d.set_item("rs1_ready", e.rs1_tag.is_none())?;
                d.set_item("rs2_ready", e.rs2_tag.is_none())?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Issue queue (Rename → Execute, waiting for operands).
    ///
    /// Listed front-to-back (oldest first). Each slot:
    /// ``{pc, raw, asm, rs1, rs2, rd, rv1, rv2, rob_tag, rs1_ready, rs2_ready}``.
    #[getter]
    fn issue_queue(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .issue_queue
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.inst.pc, e.inst.bits)?;
                d.set_item("rs1", e.inst.rs1.as_u8())?;
                d.set_item("rs2", e.inst.rs2.as_u8())?;
                d.set_item("rd", e.inst.rd.as_u8())?;
                d.set_item("rv1", e.inst.rv1)?;
                d.set_item("rv2", e.inst.rv2)?;
                d.set_item("rob_tag", e.rob_tag.0)?;
                d.set_item("rs1_ready", e.rs1_tag.is_none())?;
                d.set_item("rs2_ready", e.rs2_tag.is_none())?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Execute → Memory1 latch.
    ///
    /// Each slot: ``{pc, raw, asm, rd, alu, store_data, rob_tag}``.
    ///
    /// ``alu`` is the forwarded result available to dependent instructions.
    #[getter]
    fn execute_mem1(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .execute_mem1
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("rd", e.rd.as_u8())?;
                d.set_item("alu", e.alu)?;
                d.set_item("store_data", e.store_data)?;
                d.set_item("rob_tag", e.rob_tag.0)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Memory1 → Memory2 latch.
    ///
    /// Each slot: ``{pc, raw, asm, rd, alu, vaddr, paddr, store_data, rob_tag}``.
    #[getter]
    fn mem1_mem2(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .mem1_mem2
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("rd", e.rd.as_u8())?;
                d.set_item("alu", e.alu)?;
                d.set_item("vaddr", e.vaddr.val())?;
                d.set_item("paddr", e.paddr.val())?;
                d.set_item("store_data", e.store_data)?;
                d.set_item("rob_tag", e.rob_tag.0)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Memory2 → Writeback latch.
    ///
    /// Each slot: ``{pc, raw, asm, rd, alu, load_data, rob_tag}``.
    ///
    /// ``load_data`` carries the forwarded value for load instructions.
    #[getter]
    fn mem2_wb(&self, py: Python<'_>) -> PyResult<Py<PyList>> {
        let items: Vec<_> = self
            .inner
            .mem2_wb
            .iter()
            .map(|e| -> PyResult<_> {
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("rd", e.rd.as_u8())?;
                d.set_item("alu", e.alu)?;
                d.set_item("load_data", e.load_data)?;
                d.set_item("rob_tag", e.rob_tag.0)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
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
