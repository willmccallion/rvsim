//! The multi-letter extensions a hart implements, named as an ISA string,
//! the device tree and Linux name them.

/// A multi-letter extension the hart can implement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Extension {
    /// `cbo.clean`, `cbo.flush` and `cbo.inval`.
    Zicbom,
    /// `cbo.zero`.
    Zicboz,
    /// The `cycle`, `time` and `instret` counters.
    Zicntr,
    /// The CSR instructions.
    Zicsr,
    /// `fence.i`.
    Zifencei,
    /// The `pause` hint.
    Zihintpause,
    /// The `hpmcounter` performance counters.
    Zihpm,
    /// Half-precision floating point.
    Zfh,
    /// Address generation.
    Zba,
    /// Basic bit manipulation.
    Zbb,
    /// Carry-less multiplication.
    Zbc,
    /// Single-bit operations.
    Zbs,
    /// Bit manipulation for cryptography.
    Zbkb,
    /// Carry-less multiplication for cryptography.
    Zbkc,
    /// Crossbar permutations.
    Zbkx,
    /// Vector bit manipulation.
    Zvbb,
    /// Vector carry-less multiplication.
    Zvbc,
    /// Vector half-precision floating point.
    Zvfh,
    /// Vector bit manipulation for cryptography.
    Zvkb,
    /// Vector GCM/GMAC.
    Zvkg,
    /// Vector AES.
    Zvkned,
    /// Vector SHA-256.
    Zvknha,
    /// Vector SHA-256 and SHA-512.
    Zvknhb,
    /// Vector SM4.
    Zvksed,
    /// Vector SM3.
    Zvksh,
    /// Data-independent execution latency of the vector crypto instructions.
    Zvkt,
    /// The supervisor timer compare register.
    Sstc,
    /// Hardware updates of the PTE A and D bits.
    Svadu,
}

impl Extension {
    /// Every extension, in the order an ISA string names them: Zi, Zf, Zb,
    /// Zv, then S.
    pub const ALL: [Self; 28] = [
        Self::Zicbom,
        Self::Zicboz,
        Self::Zicntr,
        Self::Zicsr,
        Self::Zifencei,
        Self::Zihintpause,
        Self::Zihpm,
        Self::Zfh,
        Self::Zba,
        Self::Zbb,
        Self::Zbc,
        Self::Zbs,
        Self::Zbkb,
        Self::Zbkc,
        Self::Zbkx,
        Self::Zvbb,
        Self::Zvbc,
        Self::Zvfh,
        Self::Zvkb,
        Self::Zvkg,
        Self::Zvkned,
        Self::Zvknha,
        Self::Zvknhb,
        Self::Zvksed,
        Self::Zvksh,
        Self::Zvkt,
        Self::Sstc,
        Self::Svadu,
    ];

    /// The extension's lowercase name, as an ISA string spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Zicbom => "zicbom",
            Self::Zicboz => "zicboz",
            Self::Zicntr => "zicntr",
            Self::Zicsr => "zicsr",
            Self::Zifencei => "zifencei",
            Self::Zihintpause => "zihintpause",
            Self::Zihpm => "zihpm",
            Self::Zfh => "zfh",
            Self::Zba => "zba",
            Self::Zbb => "zbb",
            Self::Zbc => "zbc",
            Self::Zbs => "zbs",
            Self::Zbkb => "zbkb",
            Self::Zbkc => "zbkc",
            Self::Zbkx => "zbkx",
            Self::Zvbb => "zvbb",
            Self::Zvbc => "zvbc",
            Self::Zvfh => "zvfh",
            Self::Zvkb => "zvkb",
            Self::Zvkg => "zvkg",
            Self::Zvkned => "zvkned",
            Self::Zvknha => "zvknha",
            Self::Zvknhb => "zvknhb",
            Self::Zvksed => "zvksed",
            Self::Zvksh => "zvksh",
            Self::Zvkt => "zvkt",
            Self::Sstc => "sstc",
            Self::Svadu => "svadu",
        }
    }

    /// The single-letter extension this one builds on, which a hart
    /// without it cannot implement: F for Zfh, V for the vector
    /// extensions, S for Sstc.
    #[must_use]
    pub const fn builds_on(self) -> Option<char> {
        match self {
            Self::Zfh => Some('f'),
            Self::Zvbb
            | Self::Zvbc
            | Self::Zvfh
            | Self::Zvkb
            | Self::Zvkg
            | Self::Zvkned
            | Self::Zvknha
            | Self::Zvknhb
            | Self::Zvksed
            | Self::Zvksh
            | Self::Zvkt => Some('v'),
            Self::Sstc => Some('s'),
            _ => None,
        }
    }

    const fn bit(self) -> u32 {
        1 << (self as u32)
    }
}

/// A set of extensions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extensions(u32);

impl Extensions {
    /// No extension.
    pub const EMPTY: Self = Self(0);

    /// Every extension.
    #[must_use]
    pub fn all() -> Self {
        Extension::ALL.iter().fold(Self::EMPTY, |set, &ext| set.with(ext))
    }

    /// Whether `ext` is in the set.
    #[must_use]
    pub const fn contains(self, ext: Extension) -> bool {
        self.0 & ext.bit() != 0
    }

    /// The set with `ext`.
    #[must_use]
    pub const fn with(self, ext: Extension) -> Self {
        Self(self.0 | ext.bit())
    }

    /// The set without `ext`.
    #[must_use]
    pub const fn without(self, ext: Extension) -> Self {
        Self(self.0 & !ext.bit())
    }

    /// The extensions in the set, in ISA-string order.
    pub fn iter(self) -> impl Iterator<Item = Extension> {
        Extension::ALL.into_iter().filter(move |&ext| self.contains(ext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_extension_has_a_distinct_name_and_bit() {
        let names: std::collections::BTreeSet<&str> =
            Extension::ALL.iter().map(|ext| ext.name()).collect();
        assert_eq!(names.len(), Extension::ALL.len());
        assert!(Extension::ALL.iter().all(|ext| (ext.bit().trailing_zeros() as usize) < 32));
    }

    #[test]
    fn a_set_iterates_in_isa_string_order() {
        let set =
            Extensions::EMPTY.with(Extension::Svadu).with(Extension::Zba).with(Extension::Zvkg);

        let names: Vec<&str> = set.iter().map(Extension::name).collect();

        assert_eq!(names, ["zba", "zvkg", "svadu"]);
        assert!(!set.without(Extension::Zba).contains(Extension::Zba));
    }
}
