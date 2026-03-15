//! Register, CSR, and memory view Python bindings.
//!
//! Each view holds a `Py<PyCpu>` back-reference so reads and writes go through
//! the live CPU rather than a snapshot.

use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError};
use pyo3::prelude::*;
use rvsim_core::common::RegIdx;

use crate::cpu::PyCpu;

const fn csr_addr_to_name(addr: u64) -> Option<&'static str> {
    match addr {
        // Supervisor
        0x100 => Some("sstatus"),
        0x104 => Some("sie"),
        0x105 => Some("stvec"),
        0x140 => Some("sscratch"),
        0x141 => Some("sepc"),
        0x142 => Some("scause"),
        0x143 => Some("stval"),
        0x144 => Some("sip"),
        0x180 => Some("satp"),
        // Machine
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
        // Counters
        0xC00 => Some("cycle"),
        0xC01 => Some("time"),
        0xC02 => Some("instret"),
        0xB00 => Some("mcycle"),
        0xB02 => Some("minstret"),
        // Sstc
        0x14D => Some("stimecmp"),
        _ => None,
    }
}

/// Subscript register access returned by `cpu.regs`.
///
/// ``cpu.regs[10]`` reads x10. ``cpu.regs[10] = v`` writes x10.
#[pyclass(name = "Registers")]
pub struct Registers {
    pub cpu: Py<PyCpu>,
}

#[pymethods]
impl Registers {
    fn __getitem__(&self, py: Python<'_>, idx: usize) -> PyResult<u64> {
        if idx >= 32 {
            return Err(PyIndexError::new_err(format!("register index {idx} out of range (0–31)")));
        }
        Ok(self.cpu.borrow(py).inner.cpu.regs.read(RegIdx::new(idx as u8)))
    }

    fn __setitem__(&self, py: Python<'_>, idx: usize, value: u64) -> PyResult<()> {
        if idx >= 32 {
            return Err(PyIndexError::new_err(format!("register index {idx} out of range (0–31)")));
        }
        self.cpu.borrow_mut(py).inner.cpu.regs.write(RegIdx::new(idx as u8), value);
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let cpu = self.cpu.borrow(py);
        let vals: Vec<String> = (0u8..32)
            .filter_map(|i| {
                let v = cpu.inner.cpu.regs.read(RegIdx::new(i));
                if v != 0 { Some(format!("x{i}={v:#x}")) } else { None }
            })
            .collect();
        format!("Registers({})", vals.join(", "))
    }
}

/// Subscript CSR access returned by `cpu.csrs`.
///
/// ``cpu.csrs["mstatus"]`` or ``cpu.csrs[0x300]``.
#[pyclass(name = "Csrs")]
pub struct Csrs {
    pub cpu: Py<PyCpu>,
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
    pub cpu: Py<PyCpu>,
    pub width: u8,
}

#[pymethods]
impl Memory {
    fn __getitem__(&self, py: Python<'_>, addr: u64) -> u64 {
        let mut cpu = self.cpu.borrow_mut(py);
        let paddr = rvsim_core::common::PhysAddr::new(addr);
        match self.width {
            32 => u64::from(cpu.inner.cpu.bus.bus.read_u32(paddr)),
            64 => cpu.inner.cpu.bus.bus.read_u64(paddr),
            _ => unreachable!(),
        }
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
    pub cpu: Py<PyCpu>,
    pub width: u8,
}

#[pymethods]
impl VirtualMemory {
    fn __getitem__(&self, py: Python<'_>, addr: u64) -> PyResult<u64> {
        use rvsim_core::common::{AccessType, VirtAddr};

        let mut cpu = self.cpu.borrow_mut(py);
        let result = cpu.inner.cpu.translate(VirtAddr::new(addr), AccessType::Read, 8);
        if let Some(trap) = result.trap {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "translation failed for VA {addr:#x}: {trap:?}"
            )));
        }
        let paddr = result.paddr;
        Ok(match self.width {
            32 => u64::from(cpu.inner.cpu.bus.bus.read_u32(paddr)),
            64 => cpu.inner.cpu.bus.bus.read_u64(paddr),
            _ => unreachable!(),
        })
    }

    fn __repr__(&self) -> String {
        format!("VirtualMemory(u{})", self.width)
    }
}
