//! The host floating-point environment: exception flags and rounding mode.

use crate::isa::fp::{FpFlags, RoundingMode};

// Host FPU exception flag bits from <fenv.h> — used to detect inexact/overflow/etc.
// These are the same on x86_64 and aarch64 Linux (POSIX standard values).
pub(super) const FE_INEXACT: i32 = 0x20;

pub(super) const FE_UNDERFLOW: i32 = 0x10;

pub(super) const FE_OVERFLOW: i32 = 0x08;

pub(super) const FE_DIVBYZERO: i32 = 0x04;

pub(super) const FE_INVALID: i32 = 0x01;

pub(super) const FE_ALL_EXCEPT: i32 =
    FE_INEXACT | FE_UNDERFLOW | FE_OVERFLOW | FE_DIVBYZERO | FE_INVALID;

// Host FPU rounding-mode constants from <fenv.h>. These are platform-specific
// — SSE's MXCSR layout differs from NEON's FPCR — so they must be conditionally
// compiled per target arch.
#[cfg(target_arch = "x86_64")]
pub(super) const FE_TONEAREST: i32 = 0x0000;

#[cfg(target_arch = "x86_64")]
pub(super) const FE_DOWNWARD: i32 = 0x0400;

#[cfg(target_arch = "x86_64")]
pub(super) const FE_UPWARD: i32 = 0x0800;

#[cfg(target_arch = "x86_64")]
pub(super) const FE_TOWARDZERO: i32 = 0x0c00;

#[cfg(target_arch = "aarch64")]
pub(super) const FE_TONEAREST: i32 = 0x000000;

#[cfg(target_arch = "aarch64")]
pub(super) const FE_UPWARD: i32 = 0x400000;

#[cfg(target_arch = "aarch64")]
pub(super) const FE_DOWNWARD: i32 = 0x800000;

#[cfg(target_arch = "aarch64")]
pub(super) const FE_TOWARDZERO: i32 = 0xc00000;

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
compile_error!("rvsim FPU host-rounding-mode constants not defined for this target arch");

unsafe extern "C" {
    fn feclearexcept(excepts: i32) -> i32;
    fn fetestexcept(excepts: i32) -> i32;
    fn fesetround(round: i32) -> i32;
    fn fegetround() -> i32;
}

/// Reads and maps host FPU exception flags to RISC-V `FpFlags`.
pub fn read_host_fp_flags() -> FpFlags {
    // SAFETY: `fetestexcept` only reads this thread's FP environment.
    let host = unsafe { fetestexcept(FE_ALL_EXCEPT) };
    let mut flags = FpFlags::NONE;
    if host & FE_INVALID != 0 {
        flags = flags | FpFlags::NV;
    }
    if host & FE_DIVBYZERO != 0 {
        flags = flags | FpFlags::DZ;
    }
    if host & FE_OVERFLOW != 0 {
        flags = flags | FpFlags::OF;
    }
    if host & FE_UNDERFLOW != 0 {
        flags = flags | FpFlags::UF;
    }
    if host & FE_INEXACT != 0 {
        flags = flags | FpFlags::NX;
    }
    flags
}

/// Clears all host FPU exception flags.
pub fn clear_host_fp_flags() {
    // SAFETY: `feclearexcept` only clears this thread's FP exception flags.
    let _ = unsafe { feclearexcept(FE_ALL_EXCEPT) };
}

/// Maps a RISC-V rounding mode to the host FPU `FE_*` constant.
///
/// Neither SSE nor NEON has RMM (round to nearest, ties to max magnitude),
/// so it maps to round-to-nearest-even, and an inexact result is rounded
/// again in software by [`exact::rmm_correction`](super::exact::rmm_correction):
/// the two differ only on an exact half-ULP tie.
pub(super) const fn rm_to_host_round(rm: RoundingMode) -> i32 {
    match rm {
        RoundingMode::Rne | RoundingMode::Rmm => FE_TONEAREST,
        RoundingMode::Rtz => FE_TOWARDZERO,
        RoundingMode::Rdn => FE_DOWNWARD,
        RoundingMode::Rup => FE_UPWARD,
    }
}

/// Sets the host FPU rounding mode for a RISC-V rounding mode, returning
/// the previous host mode for later restoration.
pub fn set_host_round_mode(rm: RoundingMode) -> i32 {
    // SAFETY: `fegetround` only reads this thread's rounding mode.
    let old = unsafe { fegetround() };
    // SAFETY: `fesetround` takes an `FE_*` constant and only sets this
    // thread's rounding mode; the caller restores `old` afterwards.
    let _ = unsafe { fesetround(rm_to_host_round(rm)) };
    old
}

/// Restores the host FPU rounding mode to a previously saved value.
pub fn restore_host_round_mode(mode: i32) {
    // SAFETY: `mode` came from `fegetround`, and `fesetround` only sets this
    // thread's rounding mode.
    let _ = unsafe { fesetround(mode) };
}
