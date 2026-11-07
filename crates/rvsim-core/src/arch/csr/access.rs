//! Which CSR addresses this hart implements.

use crate::arch::Hart;
use crate::arch::pmp::PmpIdx;
use crate::isa::csr;
use crate::isa::csr::CsrAddr;

impl Hart {
    /// Returns `true` if the given CSR address corresponds to a CSR that is
    /// implemented by this hart. Accessing a non-existent CSR must raise an
    /// illegal-instruction exception (RISC-V Privileged Spec §2.2).
    pub const fn is_valid_csr(&self, addr: CsrAddr) -> bool {
        matches!(
            addr,
            csr::FFLAGS
                | csr::FRM
                | csr::FCSR
                | csr::MVENDORID
                | csr::MARCHID
                | csr::MIMPID
                | csr::MHARTID
                | csr::MSTATUS
                | csr::MEDELEG
                | csr::MIDELEG
                | csr::MIE
                | csr::MTVEC
                | csr::MISA
                | csr::MSCRATCH
                | csr::MEPC
                | csr::MCAUSE
                | csr::MTVAL
                | csr::MIP
                | csr::SSTATUS
                | csr::SIE
                | csr::STVEC
                | csr::SSCRATCH
                | csr::SEPC
                | csr::SCAUSE
                | csr::STVAL
                | csr::SIP
                | csr::STIMECMP
                | csr::SATP
                | csr::MCOUNTEREN
                | csr::SCOUNTEREN
                | csr::MENVCFG
                | csr::SENVCFG
                | csr::CYCLE
                | csr::MCYCLE
                | csr::TIME
                | csr::INSTRET
                | csr::MINSTRET
                | csr::VSTART
                | csr::VXSAT
                | csr::VXRM
                | csr::VCSR
                | csr::VL
                | csr::VTYPE
                | csr::VLENB
                | csr::CSR_SIM_PANIC
                // Sdtrig (debug trigger) stubs — read-zero / write-ignored
                | csr::TSELECT
                | csr::TDATA1
                | csr::TDATA2
                | csr::TDATA3
                | csr::TINFO
                | csr::TCONTROL
        ) || matches!(addr, csr::PMPCFG0 | csr::PMPCFG2 | csr::MCOUNTINHIBIT)
            || PmpIdx::of_pmpaddr(addr).is_some()
            || addr.is_within(csr::MHPMEVENT3, csr::MHPMEVENT31)
            || addr.is_within(csr::MHPMCOUNTER3, csr::MHPMCOUNTER31)
            || addr.is_within(csr::HPMCOUNTER3, csr::HPMCOUNTER31)
    }
}
