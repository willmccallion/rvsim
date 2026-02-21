//! Pipeline snapshot Python binding.
//!
//! Exposes `PipelineSnapshot` as a read-only Python class with built-in
//! `render()` / `visualize()` methods. All stage latches are lists of per-slot
//! dicts. No private methods are exposed.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use rvsim_core::core::pipeline::snapshot::PipelineSnapshot;
use rvsim_core::isa::disasm::disassemble;

// ── ABI register names ────────────────────────────────────────────────────────

const ABI: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];

fn reg_name(idx: usize) -> &'static str {
    ABI.get(idx).copied().unwrap_or("x?")
}

// ── Slot dict helpers ─────────────────────────────────────────────────────────

fn slot_dict<'py>(py: Python<'py>, pc: u64, raw: u32) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("pc", pc)?;
    d.set_item("raw", raw)?;
    d.set_item("asm", disassemble(raw))?;
    Ok(d)
}

// ── ASCII renderer ────────────────────────────────────────────────────────────
//
// Layout (one row per issue slot, one column per stage):
//
//   Cycle N  width=4
//        F1   F2   DE   IS   EX   M1   M2   WB
//   [0]  lui  addi slli ─    ─    ─    ─    ─
//   [1]  ─    ─    ─    ─    ─    ─    ─    ─
//        fwd: a0←0x80000  stall:M1=3

// Column width: enough for the longest common mnemonic + register operands.
const COL_W: usize = 14;
const SLOT_W: usize = 4; // "[0] "

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// One cell: mnemonic + first operand, truncated to COL_W.
fn cell(asm: &str) -> String {
    trunc(asm, COL_W)
}

/// Empty / stalled cell.
fn empty_cell(stall: u64) -> String {
    if stall > 0 {
        trunc(&format!("~{stall}"), COL_W)
    } else {
        "─".to_string()
    }
}

fn render_inner(snap: &PipelineSnapshot) -> String {
    let w = snap.width;

    // Stage definitions: (header label, per-slot cell content, stall cycles).
    // Each is a Vec<Option<String>> of length w — None means empty/stalled.
    struct StageCol {
        hdr: &'static str,
        cells: Vec<Option<String>>, // length == w
        stall: u64,
    }

    let mut cols: Vec<StageCol> = Vec::new();

    // Helper: build a StageCol from a latch vec using a closure.
    macro_rules! stage {
        ($hdr:expr, $entries:expr, $cell_fn:expr, $stall:expr) => {{
            let mut cells: Vec<Option<String>> = vec![None; w];
            for (i, e) in $entries.iter().enumerate() {
                if i < w {
                    cells[i] = Some($cell_fn(e));
                }
            }
            cols.push(StageCol {
                hdr: $hdr,
                cells,
                stall: $stall,
            });
        }};
    }

    stage!(
        "F1",
        snap.fetch1_fetch2,
        |e: &rvsim_core::core::pipeline::latches::Fetch1Fetch2Entry| { format!("{:#010x}", e.pc) },
        snap.fetch1_stall
    );

    stage!(
        "F2",
        snap.fetch2_decode,
        |e: &rvsim_core::core::pipeline::latches::IfIdEntry| { cell(&disassemble(e.inst)) },
        snap.fetch2_stall
    );

    stage!(
        "DE",
        snap.decode_rename,
        |e: &rvsim_core::core::pipeline::latches::IdExEntry| { cell(&disassemble(e.inst)) },
        0u64
    );

    stage!(
        "RN",
        snap.rename_issue,
        |e: &rvsim_core::core::pipeline::latches::RenameIssueEntry| { cell(&disassemble(e.inst)) },
        0u64
    );

    stage!(
        "IS",
        snap.issue_queue,
        |e: &rvsim_core::core::pipeline::latches::RenameIssueEntry| {
            let asm = disassemble(e.inst);
            let stalled = e.rs1_tag.is_some() || e.rs2_tag.is_some();
            if stalled {
                trunc(&format!("⋯{}", cell(&asm)), COL_W)
            } else {
                cell(&asm)
            }
        },
        0u64
    );

    stage!(
        "EX",
        snap.execute_mem1,
        |e: &rvsim_core::core::pipeline::latches::ExMem1Entry| { cell(&disassemble(e.inst)) },
        0u64
    );

    stage!(
        "M1",
        snap.mem1_mem2,
        |e: &rvsim_core::core::pipeline::latches::Mem1Mem2Entry| { cell(&disassemble(e.inst)) },
        snap.mem1_stall
    );

    stage!(
        "M2",
        snap.mem2_wb,
        |e: &rvsim_core::core::pipeline::latches::Mem2WbEntry| { cell(&disassemble(e.inst)) },
        0u64
    );

    // WB and CM: WB writes results; CM retires from ROB head.
    // Neither has an outbound latch we can inspect, so both show empty.
    {
        let cells = vec![None; w];
        cols.push(StageCol {
            hdr: "WB",
            cells,
            stall: 0,
        });
    }
    {
        let cells = vec![None; w];
        cols.push(StageCol {
            hdr: "CM",
            cells,
            stall: 0,
        });
    }

    // ── header ────────────────────────────────────────────────────────────────
    let hdr_cells: Vec<String> = cols.iter().map(|c| format!("{:^COL_W$}", c.hdr)).collect();
    let hdr_line = format!("{:SLOT_W$} {}", "", hdr_cells.join(" "));
    let rule = "─".repeat(hdr_line.len());

    // ── slot rows ─────────────────────────────────────────────────────────────
    let mut rows: Vec<String> = Vec::new();
    for slot in 0..w {
        let row_cells: Vec<String> = cols
            .iter()
            .map(|c| {
                let s = match &c.cells[slot] {
                    Some(v) => v.clone(),
                    None => empty_cell(c.stall),
                };
                format!("{s:<COL_W$}")
            })
            .collect();
        rows.push(format!("[{slot}]  {}", row_cells.join(" ")));
    }

    // ── forwarding / stall annotation line ────────────────────────────────────
    let mut notes: Vec<String> = Vec::new();
    for e in &snap.execute_mem1 {
        if e.rd != 0 {
            notes.push(format!("{}←{:#x}", reg_name(e.rd), e.alu));
        }
    }
    for e in &snap.mem2_wb {
        if e.rd != 0 {
            let v = if e.load_data != 0 { e.load_data } else { e.alu };
            notes.push(format!("{}←{:#x}", reg_name(e.rd), v));
        }
    }
    let mut stall_notes: Vec<String> = Vec::new();
    if snap.fetch1_stall > 0 {
        stall_notes.push(format!("F1={}", snap.fetch1_stall));
    }
    if snap.fetch2_stall > 0 {
        stall_notes.push(format!("F2={}", snap.fetch2_stall));
    }
    if snap.mem1_stall > 0 {
        stall_notes.push(format!("M1={}", snap.mem1_stall));
    }

    // ── assemble ──────────────────────────────────────────────────────────────
    let mut out: Vec<String> = Vec::new();
    out.push(rule.clone());
    out.push(hdr_line);
    out.push(rule.clone());
    out.extend(rows);
    if !notes.is_empty() || !stall_notes.is_empty() {
        let mut ann = String::new();
        if !notes.is_empty() {
            ann.push_str(&format!("  fwd: {}", notes.join("  ")));
        }
        if !stall_notes.is_empty() {
            if !ann.is_empty() {
                ann.push_str("  ");
            }
            ann.push_str(&format!("stall: {}", stall_notes.join(" ")));
        }
        out.push(rule.clone());
        out.push(ann);
    }
    out.push(rule);
    out.join("\n")
}

// ── PipelineSnapshot ──────────────────────────────────────────────────────────

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
///
/// Stall counters ``fetch1_stall``, ``fetch2_stall``, ``mem1_stall`` give
/// remaining hold cycles on the respective stages.
#[pyclass(name = "PipelineSnapshot", subclass)]
pub struct PyPipelineSnapshot {
    inner: PipelineSnapshot,
}

impl PyPipelineSnapshot {
    pub fn new(inner: PipelineSnapshot) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyPipelineSnapshot {
    /// Pipeline width (superscalar degree).
    #[getter]
    fn width(&self) -> usize {
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
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("rs1", e.rs1)?;
                d.set_item("rs2", e.rs2)?;
                d.set_item("rd", e.rd)?;
                d.set_item("imm", e.imm)?;
                d.set_item("rv1", e.rv1)?;
                d.set_item("rv2", e.rv2)?;
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
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("rs1", e.rs1)?;
                d.set_item("rs2", e.rs2)?;
                d.set_item("rd", e.rd)?;
                d.set_item("rv1", e.rv1)?;
                d.set_item("rv2", e.rv2)?;
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
                let d = slot_dict(py, e.pc, e.inst)?;
                d.set_item("rs1", e.rs1)?;
                d.set_item("rs2", e.rs2)?;
                d.set_item("rd", e.rd)?;
                d.set_item("rv1", e.rv1)?;
                d.set_item("rv2", e.rv2)?;
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
                d.set_item("rd", e.rd)?;
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
                d.set_item("rd", e.rd)?;
                d.set_item("alu", e.alu)?;
                d.set_item("vaddr", e.vaddr)?;
                d.set_item("paddr", e.paddr)?;
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
                d.set_item("rd", e.rd)?;
                d.set_item("alu", e.alu)?;
                d.set_item("load_data", e.load_data)?;
                d.set_item("rob_tag", e.rob_tag.0)?;
                Ok(d.into_any().unbind())
            })
            .collect::<PyResult<_>>()?;
        Ok(PyList::new(py, items)?.unbind())
    }

    /// Fetch1 stall cycles remaining (I-TLB latency).
    #[getter]
    fn fetch1_stall(&self) -> u64 {
        self.inner.fetch1_stall
    }

    /// Fetch2 stall cycles remaining (I-cache latency).
    #[getter]
    fn fetch2_stall(&self) -> u64 {
        self.inner.fetch2_stall
    }

    /// Memory1 stall cycles remaining (D-TLB / D-cache latency).
    #[getter]
    fn mem1_stall(&self) -> u64 {
        self.inner.mem1_stall
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
        let summary: Vec<String> = counts
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(name, n)| format!("{name}={n}"))
            .collect();
        format!(
            "PipelineSnapshot(width={}, {})",
            self.inner.width,
            if summary.is_empty() {
                "idle".to_string()
            } else {
                summary.join(", ")
            }
        )
    }
}
