//! The arithmetic the functional units perform, with no timing.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "register arithmetic narrows, sign-extends, reinterprets and rounds by definition"
)]

pub mod alu;

pub mod amo;

pub mod clmul;

pub mod fpu;

pub mod misaligned;

pub mod vector;
