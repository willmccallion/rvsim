//! Register, CSR, and memory view Python bindings.
//!
//! Each view holds a `Py<PySimulator>` back-reference so reads and writes go through
//! the live CPU rather than a snapshot.

use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError};
use pyo3::prelude::*;
use rvsim_core::isa::csr::CsrAddr;
use rvsim_core::isa::reg::RegIdx;

use crate::simulator::PySimulator;

/// CSR names accepted by `cpu.csrs[...]`, with their addresses.
const CSR_NAMES: &[(&str, u32)] = &[
    ("fflags", 0x001),
    ("frm", 0x002),
    ("fcsr", 0x003),
    ("vstart", 0x008),
    ("vxsat", 0x009),
    ("vxrm", 0x00A),
    ("vcsr", 0x00F),
    ("sstatus", 0x100),
    ("sie", 0x104),
    ("stvec", 0x105),
    ("scounteren", 0x106),
    ("senvcfg", 0x10A),
    ("sscratch", 0x140),
    ("sepc", 0x141),
    ("scause", 0x142),
    ("stval", 0x143),
    ("sip", 0x144),
    ("stimecmp", 0x14D),
    ("satp", 0x180),
    ("mstatus", 0x300),
    ("misa", 0x301),
    ("medeleg", 0x302),
    ("mideleg", 0x303),
    ("mie", 0x304),
    ("mtvec", 0x305),
    ("mcounteren", 0x306),
    ("menvcfg", 0x30A),
    ("mcountinhibit", 0x320),
    ("mscratch", 0x340),
    ("mepc", 0x341),
    ("mcause", 0x342),
    ("mtval", 0x343),
    ("mip", 0x344),
    ("mcycle", 0xB00),
    ("minstret", 0xB02),
    ("cycle", 0xC00),
    ("time", 0xC01),
    ("instret", 0xC02),
    ("vl", 0xC20),
    ("vtype", 0xC21),
    ("vlenb", 0xC22),
    ("mvendorid", 0xF11),
    ("marchid", 0xF12),
    ("mimpid", 0xF13),
    ("mhartid", 0xF14),
];

fn csr_name_to_addr(name: &str) -> Option<u32> {
    CSR_NAMES.iter().find(|(n, _)| *n == name).map(|(_, addr)| *addr)
}

/// Subscript register access returned by `cpu.regs` and `cpu.harts[n].regs`.
///
/// ``cpu.harts[0].regs[10]`` reads x10. ``cpu.harts[0].regs[10] = v`` writes x10.
#[pyclass(name = "Registers")]
pub struct Registers {
    pub cpu: Py<PySimulator>,
    pub hart: usize,
}

#[pymethods]
impl Registers {
    fn __getitem__(&self, py: Python<'_>, idx: usize) -> PyResult<u64> {
        if idx >= 32 {
            return Err(PyIndexError::new_err(format!("register index {idx} out of range (0–31)")));
        }
        Ok(self.cpu.borrow(py).inner.state.harts[self.hart].regs.read(RegIdx::new(idx as u8)))
    }

    fn __setitem__(&self, py: Python<'_>, idx: usize, value: u64) -> PyResult<()> {
        if idx >= 32 {
            return Err(PyIndexError::new_err(format!("register index {idx} out of range (0–31)")));
        }
        self.cpu.borrow_mut(py).inner.state.harts[self.hart]
            .regs
            .write(RegIdx::new(idx as u8), value);
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let cpu = self.cpu.borrow(py);
        let vals: Vec<String> = (0u8..32)
            .filter_map(|i| {
                let v = cpu.inner.state.harts[self.hart].regs.read(RegIdx::new(i));
                if v != 0 { Some(format!("x{i}={v:#x}")) } else { None }
            })
            .collect();
        format!("Registers({})", vals.join(", "))
    }
}

/// Subscript CSR access returned by `cpu.csrs` and `cpu.harts[n].csrs`.
///
/// ``cpu.harts[0].csrs["mstatus"]`` or ``cpu.harts[0].csrs[0x300]``.
#[pyclass(name = "Csrs")]
pub struct Csrs {
    pub cpu: Py<PySimulator>,
    pub hart: usize,
}

#[pymethods]
impl Csrs {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<u64> {
        let addr = if let Ok(addr) = key.extract::<u32>() {
            addr
        } else if let Ok(name) = key.extract::<String>() {
            csr_name_to_addr(&name.to_lowercase())
                .ok_or_else(|| PyKeyError::new_err(format!("unknown CSR name {name:?}")))?
        } else {
            return Err(PyTypeError::new_err("CSR key must be a str or int"));
        };
        self.cpu
            .borrow_mut(py)
            .read_csr(self.hart, CsrAddr::from_u32(addr))
            .ok_or_else(|| PyKeyError::new_err(format!("CSR {addr:#x} is not implemented")))
    }

    const fn __repr__(&self) -> &'static str {
        "Csrs(...)"
    }
}

/// One hart's architectural state, returned by `cpu.harts[n]`.
#[pyclass(name = "Hart")]
pub struct Hart {
    pub cpu: Py<PySimulator>,
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
        self.cpu.borrow(py).inner.state.harts[self.index].pc
    }

    #[setter]
    fn set_pc(&self, py: Python<'_>, value: u64) {
        self.cpu.borrow_mut(py).inner.set_pc(self.index, value);
    }

    /// Current privilege level: ``"M"``, ``"S"``, or ``"U"``.
    #[getter]
    fn privilege(&self, py: Python<'_>) -> &'static str {
        self.cpu.borrow(py).privilege_str(self.index)
    }

    /// Instructions this hart has retired.
    #[getter]
    fn instructions_retired(&self, py: Python<'_>) -> u64 {
        self.cpu.borrow(py).inner.state.harts[self.index].instructions_retired
    }

    /// Register file — ``cpu.harts[n].regs[10]``.
    #[getter]
    fn regs(&self, py: Python<'_>) -> Registers {
        Registers { cpu: self.cpu.clone_ref(py), hart: self.index }
    }

    /// CSRs — ``cpu.harts[n].csrs["mstatus"]``.
    #[getter]
    fn csrs(&self, py: Python<'_>) -> Csrs {
        Csrs { cpu: self.cpu.clone_ref(py), hart: self.index }
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let cpu = self.cpu.borrow(py);
        let hart = &cpu.inner.state.harts[self.index];
        format!(
            "Hart(id={}, pc={:#x}, privilege={})",
            self.index,
            hart.pc,
            cpu.privilege_str(self.index)
        )
    }
}

/// The system's harts, returned by `cpu.harts`.
#[pyclass(name = "Harts")]
pub struct Harts {
    pub cpu: Py<PySimulator>,
}

#[pymethods]
impl Harts {
    fn __len__(&self, py: Python<'_>) -> usize {
        self.cpu.borrow(py).inner.state.harts.len()
    }

    fn __getitem__(&self, py: Python<'_>, index: isize) -> PyResult<Hart> {
        let count = self.__len__(py);
        let resolved = if index < 0 { index + count as isize } else { index };
        if resolved < 0 || resolved as usize >= count {
            return Err(PyIndexError::new_err(format!(
                "hart index {index} out of range (0–{})",
                count - 1
            )));
        }
        Ok(Hart { cpu: self.cpu.clone_ref(py), index: resolved as usize })
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!("Harts({})", self.__len__(py))
    }
}

/// Subscript memory access returned by `cpu.mem32` or `cpu.mem64`.
///
/// ``cpu.mem32[addr]`` reads a u32. ``cpu.mem64[addr]`` reads a u64.
/// These use **physical** addresses — no MMU translation.
#[pyclass(name = "Memory")]
pub struct Memory {
    pub cpu: Py<PySimulator>,
    pub width: u8,
}

#[pymethods]
impl Memory {
    fn __getitem__(&self, py: Python<'_>, addr: u64) -> u64 {
        let mut cpu = self.cpu.borrow_mut(py);
        let paddr = rvsim_core::common::PhysAddr::new(addr);
        let width = match self.width {
            32 => 4,
            64 => 8,
            _ => unreachable!(),
        };
        cpu.inner.probe_mem_load(paddr, width)
    }

    fn __repr__(&self) -> String {
        format!("Memory(u{})", self.width)
    }
}

/// Subscript memory access with virtual-to-physical translation via the MMU.
///
/// ``cpu.vmem64[addr]`` translates `addr` through the current page tables
/// (using SATP), then reads the resulting physical address.
/// Returns 0 if translation fails (page fault).
#[pyclass(name = "VirtualMemory")]
pub struct VirtualMemory {
    pub cpu: Py<PySimulator>,
    pub width: u8,
}

#[pymethods]
impl VirtualMemory {
    fn __getitem__(&self, py: Python<'_>, addr: u64) -> PyResult<u64> {
        use rvsim_core::common::{AccessType, VirtAddr};
        use rvsim_core::uarch::mmu::TranslateOutcome;

        let mut cpu = self.cpu.borrow_mut(py);
        // FFI-boundary translate: synchronously drive the walk inline,
        // because the Python caller can't park.
        let mut outcome =
            cpu.inner.state.core_ctx(0).translate(VirtAddr::new(addr), AccessType::Read, 8);
        let paddr = loop {
            match outcome {
                TranslateOutcome::Ready(result) => {
                    if let Some(trap) = result.trap {
                        return Err(pyo3::exceptions::PyValueError::new_err(format!(
                            "translation failed for VA {addr:#x}: {trap:?}"
                        )));
                    }
                    break result.paddr;
                }
                TranslateOutcome::NeedPte { pte_addr, state } => {
                    let raw_pte = cpu.inner.probe_mem_load(pte_addr, 8);
                    outcome = cpu.inner.state.core_ctx(0).translate_continue(state, raw_pte, 0);
                }
            }
        };

        let width = match self.width {
            32 => 4,
            64 => 8,
            _ => unreachable!(),
        };
        Ok(cpu.inner.probe_mem_load(paddr, width))
    }

    fn __repr__(&self) -> String {
        format!("VirtualMemory(u{})", self.width)
    }
}
