//! The Python package's enums, whose members the extension returns.

use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;
use rvsim_core::isa::privileged::PrivilegeMode;

static STOP_REASON: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static DEBUG_END: PyOnceLock<Py<PyType>> = PyOnceLock::new();
static PRIVILEGE: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// The member of the enum `module.name`, looked up once, whose value is
/// `value`.
fn member<'py>(
    enum_type: &PyOnceLock<Py<PyType>>,
    py: Python<'py>,
    (module, name): (&str, &str),
    value: &str,
) -> PyResult<Bound<'py, PyAny>> {
    enum_type.import(py, module, name)?.call1((value,))
}

/// `rvsim.StopReason`'s member whose value is `value`.
pub fn stop_reason<'py>(py: Python<'py>, value: &str) -> PyResult<Bound<'py, PyAny>> {
    member(&STOP_REASON, py, ("rvsim.simulator", "StopReason"), value)
}

/// `rvsim.DebugEnd`'s member whose value is `value`.
pub fn debug_end<'py>(py: Python<'py>, value: &str) -> PyResult<Bound<'py, PyAny>> {
    member(&DEBUG_END, py, ("rvsim.simulator", "DebugEnd"), value)
}

/// `rvsim.Privilege`'s member for `mode`.
pub fn privilege(py: Python<'_>, mode: PrivilegeMode) -> PyResult<Bound<'_, PyAny>> {
    member(&PRIVILEGE, py, ("rvsim.isa", "Privilege"), privilege_letter(mode))
}

/// The letter the spec names `mode` by.
pub const fn privilege_letter(mode: PrivilegeMode) -> &'static str {
    match mode {
        PrivilegeMode::Machine => "M",
        PrivilegeMode::Supervisor => "S",
        PrivilegeMode::User => "U",
    }
}
