//! ISA capability configuration.
//!
//! Describes what RISC-V extensions and per-extension parameters this
//! implementation supports. Stored on [`Config`](crate::config::Config) and
//! accessed via `soc.config.isa` at runtime. As the simulator grows
//! per-extension gating (Zvk*, Zvfbfmin, Sstc, H, …), new fields and
//! sub-structs are added here, grouped by extension family.
//!
//! Currently only the vector family has runtime gating; other extensions
//! (M/A/F/D/C/Zicboz/Zk*/Zb*/…) are always implemented at full strength.
//! The struct documents the intent so future gating has an obvious home.

use serde::Deserialize;

/// Top-level ISA capability descriptor.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsaConfig {
    /// Vector extension capabilities.
    #[serde(default)]
    pub vector: VectorIsa,

    /// Whether the Svadu extension is implemented: `menvcfg.ADUE` is then
    /// writable and, while set, the page-table walker updates PTE A/D bits
    /// itself. Without it, or with ADUE clear, the hart behaves as Svade and
    /// raises a page fault when an A/D bit needs setting.
    #[serde(default)]
    pub svadu: bool,
}

/// Vector ISA capabilities (RVV 1.0 and sub-extensions).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorIsa {
    /// Maximum vector element width in bits (`ELEN`). Spec values: 32 (`Zve32x`)
    /// or 64 (`Zve64x`). `vsetvl` rejects requested `SEW > ELEN` with `vill = 1`.
    #[serde(default = "VectorIsa::default_elen")]
    pub elen: usize,

    /// Whether the Zvfh (half-precision vector floating-point) extension is
    /// supported. When false, vector ops with `SEW = 16` return reserved
    /// behaviour (the simulator falls through unsupported match arms).
    #[serde(default = "VectorIsa::default_zvfh")]
    pub zvfh: bool,
}

impl VectorIsa {
    const fn default_elen() -> usize {
        64
    }

    const fn default_zvfh() -> bool {
        true
    }
}

impl Default for VectorIsa {
    fn default() -> Self {
        Self { elen: Self::default_elen(), zvfh: Self::default_zvfh() }
    }
}
