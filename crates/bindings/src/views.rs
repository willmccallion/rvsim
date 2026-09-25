//! Register, CSR, and memory view Python bindings.
//!
//! Each view holds a `Py<PySimulator>` back-reference so reads and writes go through
//! the live CPU rather than a snapshot.

use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError};
use pyo3::prelude::*;
use rvsim_core::common::RegIdx;

use crate::simulator::PySimulator;

const fn csr_addr_to_name(addr: u64) -> Option<&'static str> {
    match addr {
        0x100 => Some("sstatus"),
        0x104 => Some("sie"),
        0x105 => Some("stvec"),
        0x140 => Some("sscratch"),
        0x141 => Some("sepc"),
        0x142 => Some("scause"),
        0x143 => Some("stval"),
        0x144 => Some("sip"),
        0x180 => Some("satp"),
        0x300 => Some("mstatus"),
        0x301 => Some("misa"),
        0x302 => Some("medeleg"),
        0x303 => Some("mideleg"),
        0x304 => Some("mie"),
        0x305 => Some("mtvec"),
        0x340 => Some("mscratch"),
        0x341 => Some("mepc"),
        0x342 => Some("mcause"),
        0x343 => Some("mtval"),
        0x344 => Some("mip"),
        0xC00 => Some("cycle"),
        0xC01 => Some("time"),
        0xC02 => Some("instret"),
        0xB00 => Some("mcycle"),
        0xB02 => Some("minstret"),
        0x14D => Some("stimecmp"),
        _ => None,
    }
}

/// Subscript register access returned by `cpu.harts[0].regs`.
///
/// ``cpu.harts[0].regs[10]`` reads x10. ``cpu.harts[0].regs[10] = v`` writes x10.
#[pyclass(name = "Registers")]
pub struct Registers {
    pub cpu: Py<PySimulator>,
}

#[pymethods]
impl Registers {
    fn __getitem__(&self, py: Python<'_>, idx: usize) -> PyResult<u64> {
        if idx >= 32 {
            return Err(PyIndexError::new_err(format!("register index {idx} out of range (0–31)")));
        }
        Ok(self.cpu.borrow(py).inner.state.harts[0].regs.read(RegIdx::new(idx as u8)))
    }

    fn __setitem__(&self, py: Python<'_>, idx: usize, value: u64) -> PyResult<()> {
        if idx >= 32 {
            return Err(PyIndexError::new_err(format!("register index {idx} out of range (0–31)")));
        }
        self.cpu.borrow_mut(py).inner.state.harts[0].regs.write(RegIdx::new(idx as u8), value);
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let cpu = self.cpu.borrow(py);
        let vals: Vec<String> = (0u8..32)
            .filter_map(|i| {
                let v = cpu.inner.state.harts[0].regs.read(RegIdx::new(i));
                if v != 0 { Some(format!("x{i}={v:#x}")) } else { None }
            })
            .collect();
        format!("Registers({})", vals.join(", "))
    }
}

/// Subscript CSR access returned by `cpu.harts[0].csrs`.
///
/// ``cpu.harts[0].csrs["mstatus"]`` or ``cpu.harts[0].csrs[0x300]``.
#[pyclass(name = "Csrs")]
pub struct Csrs {
    pub cpu: Py<PySimulator>,
}

#[pymethods]
impl Csrs {
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Option<u64>> {
        let name: String = if let Ok(addr) = key.extract::<u64>() {
            csr_addr_to_name(addr)
                .ok_or_else(|| PyKeyError::new_err(format!("unknown CSR address {addr:#x}")))?
                .to_string()
        } else if let Ok(s) = key.extract::<String>() {
            s.to_lowercase()
        } else {
            return Err(PyTypeError::new_err("CSR key must be a str or int"));
        };
        Ok(self.cpu.borrow(py).read_csr_by_name(&name))
    }

    const fn __repr__(&self) -> &'static str {
        "Csrs(...)"
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
        use rvsim_core::sim::state::memory::TranslateResult;

        let mut cpu = self.cpu.borrow_mut(py);
        // FFI-boundary translate: synchronously drive the walk inline,
        // because the Python caller can't park.
        let mut outcome = cpu.inner.state.core_ctx(0).translate(VirtAddr::new(addr), AccessType::Read, 8);
        let paddr = loop {
            match outcome {
                TranslateResult::Ready(result) => {
                    if let Some(trap) = result.trap {
                        return Err(pyo3::exceptions::PyValueError::new_err(format!(
                            "translation failed for VA {addr:#x}: {trap:?}"
                        )));
                    }
                    break result.paddr;
                }
                TranslateResult::NeedPte { pte_addr, state } => {
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
