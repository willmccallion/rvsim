//! Register writes through the Python views reach the running backend.

use pyo3::prelude::*;
use rvsim_core::Simulator;
use rvsim_core::common::PhysAddr;
use rvsim_core::config::{BackendKind, Config, Console};

use crate::simulator::PySimulator;
use crate::views::Registers;

const PROGRAM_BASE: u64 = 0x8000_0000;
/// `addi a1, a0, 0`.
const COPY_A0_TO_A1: u32 = 0x0005_0593;
const JAL_SELF: u32 = 0x0000_006F;
const A0: usize = 10;
const A1: usize = 11;
const WRITTEN: u64 = 42;

/// What `a1` holds after Python wrote `a0` through `regs[10] = 42` and the
/// program copied `a0` into `a1`.
fn copied_after_python_write(backend: BackendKind) -> u64 {
    pyo3::prepare_freethreaded_python();
    Python::with_gil(|py| {
        let mut config = Config::minimal();
        config.pipeline.backend = backend;
        config.system.console = Console::Quiet;
        let mut inner = Simulator::build(&config, "").expect("the test machine builds");
        for (n, word) in [COPY_A0_TO_A1, JAL_SELF].into_iter().enumerate() {
            inner.probe_mem_store(PhysAddr::new(PROGRAM_BASE + 4 * n as u64), u64::from(word), 4);
        }
        inner.set_pc(0, PROGRAM_BASE);
        let cpu = Py::new(py, PySimulator { inner }).expect("simulator object");
        let regs = Py::new(py, Registers { cpu: cpu.clone_ref(py), hart: 0 }).expect("view");
        regs.bind(py).set_item(A0, WRITTEN).expect("regs[10] = 42");

        for _ in 0..200 {
            cpu.borrow_mut(py).inner.tick().expect("tick");
        }

        regs.bind(py).get_item(A1).and_then(|v| v.extract::<u64>()).expect("regs[11]")
    })
}

#[test]
fn a_register_written_from_python_is_read_by_the_next_instruction_o3() {
    let copied = copied_after_python_write(BackendKind::OutOfOrder);

    assert_eq!(copied, WRITTEN);
}

#[test]
fn a_register_written_from_python_is_read_by_the_next_instruction_inorder() {
    let copied = copied_after_python_write(BackendKind::InOrder);

    assert_eq!(copied, WRITTEN);
}
