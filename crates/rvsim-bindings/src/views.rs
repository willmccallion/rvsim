//! Register, CSR, and memory view Python bindings.
//!
//! Each view holds a `Py<PySimulator>` back-reference so reads and writes go through
//! the live CPU rather than a snapshot.

use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError};
use pyo3::prelude::*;
use rvsim_core::common::{PhysAddr, VirtAddr};
use rvsim_core::isa::csr::{self, CsrAddr};
use rvsim_core::isa::reg::RegIdx;
use rvsim_core::system::simulator::ProbeWidth;

use crate::python_enums;
use crate::simulator::PySimulator;

/// The CSR `key` names: an address, or a name the privileged spec gives it.
fn csr_address(key: &Bound<'_, PyAny>) -> PyResult<CsrAddr> {
    if let Ok(addr) = key.extract::<u64>() {
        return u16::try_from(addr)
            .ok()
            .filter(|&addr| addr <= MAX_CSR_ADDRESS)
            .map(CsrAddr::new)
            .ok_or_else(|| PyKeyError::new_err(format!("{addr:#x} is not a 12-bit CSR address")));
    }
    let name: String =
        key.extract().map_err(|_| PyTypeError::new_err("a CSR key is a name or an address"))?;
    csr::named(&name.to_lowercase())
        .ok_or_else(|| PyKeyError::new_err(format!("unknown CSR name {name:?}")))
}

/// The highest CSR address, in 12 bits.
const MAX_CSR_ADDRESS: u16 = 0xFFF;

/// Subscript register access returned by `sim.regs` and `sim.harts[n].regs`.
///
/// ``sim.harts[0].regs[10]`` reads x10. ``sim.harts[0].regs[10] = v`` writes x10
/// and drops what the pipeline has in flight, as setting the PC does.
#[pyclass(name = "Registers", module = "rvsim._core")]
#[derive(Debug)]
pub struct Registers {
    /// The simulator the registers belong to.
    pub sim: Py<PySimulator>,
    /// The hart whose registers these are.
    pub hart: usize,
}

/// The integer register Python's `idx` names, or an `IndexError`.
fn register_index(idx: usize) -> PyResult<RegIdx> {
    u8::try_from(idx).ok().filter(|&i| usize::from(i) < RegIdx::COUNT).map(RegIdx::new).ok_or_else(
        || {
            PyIndexError::new_err(format!(
                "register index {idx} out of range (0–{})",
                RegIdx::COUNT - 1
            ))
        },
    )
}

#[pymethods]
impl Registers {
    fn __getitem__(&self, py: Python<'_>, idx: usize) -> PyResult<u64> {
        Ok(self.sim.borrow(py).inner.hart(self.hart).regs.read(register_index(idx)?))
    }

    fn __setitem__(&self, py: Python<'_>, idx: usize, value: u64) -> PyResult<()> {
        self.sim.borrow_mut(py).inner.set_reg(self.hart, register_index(idx)?, value);
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let sim = self.sim.borrow(py);
        let regs = &sim.inner.hart(self.hart).regs;
        let vals: Vec<String> = RegIdx::all()
            .filter_map(|reg| {
                let v = regs.read(reg);
                (v != 0).then(|| format!("{reg}={v:#x}"))
            })
            .collect();
        format!("Registers({})", vals.join(", "))
    }
}

/// Subscript CSR access returned by `sim.csrs` and `sim.harts[n].csrs`.
///
/// ``sim.harts[0].csrs["mstatus"]`` or ``sim.harts[0].csrs[0x300]``.
#[pyclass(name = "Csrs", module = "rvsim._core")]
#[derive(Debug)]
pub struct Csrs {
    /// The simulator the CSRs belong to.
    pub sim: Py<PySimulator>,
    /// The hart whose CSRs these are.
    pub hart: usize,
}

#[pymethods]
impl Csrs {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<u64> {
        let addr = csr_address(key)?;
        self.sim.borrow_mut(py).inner.read_csr(self.hart, addr).ok_or_else(|| {
            PyKeyError::new_err(format!("CSR {:#x} is not implemented", addr.as_u32()))
        })
    }

    fn __repr__(&self) -> String {
        format!("Csrs(hart={})", self.hart)
    }
}

/// One hart's architectural state, returned by `sim.harts[n]`.
#[pyclass(name = "Hart", module = "rvsim._core")]
#[derive(Debug)]
pub struct Hart {
    /// The simulator the hart belongs to.
    pub sim: Py<PySimulator>,
    /// The hart's index.
    pub index: usize,
}

#[pymethods]
impl Hart {
    /// Hart id.
    #[getter]
    const fn id(&self) -> usize {
        self.index
    }

    /// The architectural PC (the next instruction to retire). Writing it
    /// drops everything in flight and restarts fetch there.
    #[getter]
    fn pc(&self, py: Python<'_>) -> u64 {
        self.sim.borrow(py).inner.hart(self.index).pc.val()
    }

    #[setter]
    fn set_pc(&self, py: Python<'_>, value: u64) {
        self.sim.borrow_mut(py).inner.set_pc(self.index, VirtAddr::new(value));
    }

    /// The hart's privilege mode, a `Privilege`.
    #[getter]
    fn privilege<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        python_enums::privilege(py, self.sim.borrow(py).inner.hart(self.index).privilege)
    }

    /// Instructions this hart has retired.
    #[getter]
    fn instructions_retired(&self, py: Python<'_>) -> u64 {
        self.sim.borrow(py).inner.hart(self.index).instructions_retired
    }

    /// Register file — ``sim.harts[n].regs[10]``.
    #[getter]
    fn regs(&self, py: Python<'_>) -> Registers {
        Registers { sim: self.sim.clone_ref(py), hart: self.index }
    }

    /// CSRs — ``sim.harts[n].csrs["mstatus"]``.
    #[getter]
    fn csrs(&self, py: Python<'_>) -> Csrs {
        Csrs { sim: self.sim.clone_ref(py), hart: self.index }
    }

    /// 32-bit reads through this hart's page tables —
    /// ``sim.harts[n].vmem32[vaddr]``.
    #[getter]
    fn vmem32(&self, py: Python<'_>) -> VirtualMemory {
        VirtualMemory { sim: self.sim.clone_ref(py), hart: self.index, width: Width::Bits32 }
    }

    /// 64-bit reads through this hart's page tables —
    /// ``sim.harts[n].vmem64[vaddr]``.
    #[getter]
    fn vmem64(&self, py: Python<'_>) -> VirtualMemory {
        VirtualMemory { sim: self.sim.clone_ref(py), hart: self.index, width: Width::Bits64 }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let sim = self.sim.borrow(py);
        let hart = sim.inner.hart(self.index);
        format!(
            "Hart(id={}, pc={:#x}, privilege={})",
            self.index,
            hart.pc,
            python_enums::privilege_letter(hart.privilege)
        )
    }
}

/// The system's harts, returned by `sim.harts`.
#[pyclass(name = "Harts", module = "rvsim._core")]
#[derive(Debug)]
pub struct Harts {
    /// The simulator the harts belong to.
    pub sim: Py<PySimulator>,
}

#[pymethods]
impl Harts {
    fn __len__(&self, py: Python<'_>) -> usize {
        self.sim.borrow(py).inner.hart_count()
    }

    fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<Hart> {
        let count = self.__len__(py);
        let resolved = if index < 0 { index + count.cast_signed() } else { index };
        if resolved < 0 || resolved.cast_unsigned() >= count {
            return Err(PyIndexError::new_err(format!(
                "hart index {index} out of range (0–{})",
                count - 1
            )));
        }
        Ok(Hart { sim: self.sim.clone_ref(py), index: resolved.cast_unsigned() })
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!("Harts({})", self.__len__(py))
    }
}

/// How many bits one memory read returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    /// A 32-bit read.
    Bits32,
    /// A 64-bit read.
    Bits64,
}

impl Width {
    const fn probe_width(self) -> ProbeWidth {
        match self {
            Self::Bits32 => ProbeWidth::B4,
            Self::Bits64 => ProbeWidth::B8,
        }
    }

    const fn bits(self) -> u8 {
        match self {
            Self::Bits32 => 32,
            Self::Bits64 => 64,
        }
    }
}

/// Subscript memory access returned by `sim.mem32` or `sim.mem64`.
///
/// ``sim.mem32[addr]`` reads a u32. ``sim.mem64[addr]`` reads a u64.
/// These use **physical** addresses — no MMU translation.
#[pyclass(name = "Memory", module = "rvsim._core")]
#[derive(Debug)]
pub struct Memory {
    /// The simulator whose memory this reads.
    pub sim: Py<PySimulator>,
    /// Bits per read.
    pub width: Width,
}

#[pymethods]
impl Memory {
    fn __getitem__(&self, py: Python<'_>, addr: u64) -> u64 {
        let mut sim = self.sim.borrow_mut(py);
        sim.inner.probe_mem_load(PhysAddr::new(addr), self.width.probe_width())
    }

    fn __repr__(&self) -> String {
        format!("Memory(u{})", self.width.bits())
    }
}

/// Subscript memory access with virtual-to-physical translation via the MMU.
///
/// ``sim.harts[n].vmem64[addr]`` translates `addr` through hart `n`'s page
/// tables (its SATP), then reads the resulting physical address;
/// ``sim.vmem64`` is hart 0's. A failed translation raises ``ValueError``.
#[pyclass(name = "VirtualMemory", module = "rvsim._core")]
#[derive(Debug)]
pub struct VirtualMemory {
    /// The simulator whose memory this reads.
    pub sim: Py<PySimulator>,
    /// The hart whose page tables translate.
    pub hart: usize,
    /// Bits per read.
    pub width: Width,
}

#[pymethods]
impl VirtualMemory {
    fn __getitem__(&self, py: Python<'_>, addr: u64) -> PyResult<u64> {
        let mut sim = self.sim.borrow_mut(py);
        let paddr = sim.inner.translate_now(self.hart, VirtAddr::new(addr)).map_err(|trap| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "translation failed for VA {addr:#x}: {trap:?}"
            ))
        })?;
        Ok(sim.inner.probe_mem_load(paddr, self.width.probe_width()))
    }

    fn __repr__(&self) -> String {
        format!("VirtualMemory(u{}, hart={})", self.width.bits(), self.hart)
    }
}
