//! The multi-letter extensions a hart implements, named as an ISA string,
//! the device tree and Linux name them.

use crate::isa::misa::{IsaStringError, Misa};

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

/// What an instruction needs the hart to implement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The base ISA, which every hart has.
    Base,
    /// The single-letter extension (lowercase) of `misa`.
    Letter(char),
    /// Any one of these extensions, each of which has the instruction.
    AnyOf(Extensions),
}

impl Requirement {
    /// Any one of `extensions`.
    #[must_use]
    pub fn any_of(extensions: &[Extension]) -> Self {
        Self::AnyOf(extensions.iter().fold(Extensions::EMPTY, |set, &ext| set.with(ext)))
    }
}

/// The ISA a hart implements: its `misa` letters and its multi-letter
/// extensions, which decode checks instructions against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Isa {
    /// The single-letter extensions.
    pub misa: Misa,
    /// The multi-letter extensions.
    pub extensions: Extensions,
}

/// An ISA string parsed: the ISA it names, and the vector register length
/// it names with `zvl<bits>b`, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedIsa {
    /// The ISA.
    pub isa: Isa,
    /// The VLEN named, to check against the vector unit's.
    pub vlen: Option<usize>,
}

/// The extensions every hart implements, which an ISA string need not name.
const ALWAYS: [Extension; 5] = [
    Extension::Zicsr,
    Extension::Zifencei,
    Extension::Zicntr,
    Extension::Zihintpause,
    Extension::Zihpm,
];

/// The shorthand names an ISA string may use for groups of vector crypto
/// extensions, and what each stands for.
const GROUPS: [(&str, &[Extension]); 6] = [
    (
        "zvkn",
        &[
            Extension::Zvkned,
            Extension::Zvknha,
            Extension::Zvknhb,
            Extension::Zvkb,
            Extension::Zvkt,
        ],
    ),
    (
        "zvknc",
        &[
            Extension::Zvkned,
            Extension::Zvknha,
            Extension::Zvknhb,
            Extension::Zvkb,
            Extension::Zvkt,
            Extension::Zvbc,
        ],
    ),
    (
        "zvkng",
        &[
            Extension::Zvkned,
            Extension::Zvknha,
            Extension::Zvknhb,
            Extension::Zvkb,
            Extension::Zvkt,
            Extension::Zvkg,
        ],
    ),
    ("zvks", &[Extension::Zvksed, Extension::Zvksh, Extension::Zvkb, Extension::Zvkt]),
    (
        "zvksc",
        &[Extension::Zvksed, Extension::Zvksh, Extension::Zvkb, Extension::Zvkt, Extension::Zvbc],
    ),
    (
        "zvksg",
        &[Extension::Zvksed, Extension::Zvksh, Extension::Zvkb, Extension::Zvkt, Extension::Zvkg],
    ),
];

impl Isa {
    /// Everything the simulator implements: RV64IMAFDCB, V when
    /// `with_v`, and every multi-letter extension that builds on them.
    #[must_use]
    pub fn everything(with_v: bool) -> Self {
        let misa = Misa::rv64gcb(with_v);
        let extensions = Extension::ALL
            .into_iter()
            .filter(|ext| ext.builds_on().is_none_or(|letter| misa.has(letter)))
            .fold(Extensions::EMPTY, Extensions::with);
        Self { misa, extensions }
    }

    /// Parses an ISA string such as `rv64gcv_zba_zbb_zbs_zicbom`: the
    /// single letters (`g` is `imafd`, `b` is Zba, Zbb and Zbs), then the
    /// multi-letter extensions and vector crypto groups (`zvkn`, `zvks`,
    /// ...) the hart implements, each needing the letter it builds on.
    /// Zicsr, Zifencei, Zicntr, Zihintpause, Zihpm, Sstc and, with V,
    /// Zvkt are always implemented and need not be named.
    ///
    /// # Errors
    ///
    /// [`IsaStringError`] for a string that does not describe a hart this
    /// simulator implements.
    pub fn parse(text: &str) -> Result<ParsedIsa, IsaStringError> {
        let lower = text.to_ascii_lowercase();
        let (letters, names) = lower.split_once('_').unwrap_or((&lower, ""));
        let mut misa: Misa = letters.parse().map_err(|e: IsaStringError| e.for_string(text))?;
        let mut extensions = if misa.has('b') {
            Requirement::any_of(&[Extension::Zba, Extension::Zbb, Extension::Zbs])
        } else {
            Requirement::Base
        };
        let mut set = match extensions {
            Requirement::AnyOf(set) => set,
            _ => Extensions::EMPTY,
        };
        let mut vlen = None;
        for name in names.split('_').filter(|name| !name.is_empty()) {
            if let Some(bits) = name.strip_prefix("zvl").and_then(|rest| rest.strip_suffix('b')) {
                vlen = Some(bits.parse().map_err(|_| {
                    IsaStringError::MalformedVlen(text.to_owned(), name.to_owned())
                })?);
                continue;
            }
            if name == "zve64d" {
                continue;
            }
            if let Some((_, members)) = GROUPS.iter().find(|(group, _)| *group == name) {
                set = members.iter().fold(set, |set, &ext| set.with(ext));
                continue;
            }
            let ext =
                Extension::ALL.into_iter().find(|ext| ext.name() == name).ok_or_else(|| {
                    IsaStringError::UnknownExtension(text.to_owned(), name.to_owned())
                })?;
            set = set.with(ext);
        }
        extensions = Requirement::Base;
        let _ = extensions;
        for ext in ALWAYS {
            set = set.with(ext);
        }
        set = set.with(Extension::Sstc);
        if misa.has('v') {
            set = set.with(Extension::Zvkt);
        }
        for ext in set.iter() {
            if let Some(letter) = ext.builds_on()
                && !misa.has(letter)
            {
                return Err(IsaStringError::Requires(
                    text.to_owned(),
                    ext.name().to_owned(),
                    letter,
                ));
            }
        }
        let has_b =
            [Extension::Zba, Extension::Zbb, Extension::Zbs].iter().all(|&ext| set.contains(ext));
        misa = misa.with_b(has_b);
        Ok(ParsedIsa { isa: Self { misa, extensions: set }, vlen })
    }

    /// Whether the hart implements what an instruction needs.
    #[must_use]
    pub fn permits(&self, requirement: Requirement) -> bool {
        match requirement {
            Requirement::Base => true,
            Requirement::Letter(letter) => self.misa.has(letter),
            Requirement::AnyOf(extensions) => self.extensions.intersects(extensions),
        }
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

    /// Whether the set has any of `others`.
    #[must_use]
    pub const fn intersects(self, others: Self) -> bool {
        self.0 & others.0 != 0
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

    fn parsed(text: &str) -> Isa {
        Isa::parse(text).expect("a valid ISA string").isa
    }

    #[test]
    fn letters_alone_give_the_always_implemented_extensions_and_no_more() {
        let isa = parsed("rv64gc");

        let names: Vec<&str> = isa.extensions.iter().map(Extension::name).collect();
        assert_eq!(names, ["zicntr", "zicsr", "zifencei", "zihintpause", "zihpm", "sstc"]);
        assert!(!isa.misa.has('b') && !isa.misa.has('v'));
    }

    #[test]
    fn b_is_the_three_bit_manipulation_extensions_together() {
        assert!(parsed("rv64gcb").extensions.contains(Extension::Zbs));
        assert!(parsed("rv64gc_zba_zbb_zbs").misa.has('b'));
        assert!(!parsed("rv64gc_zba_zbb").misa.has('b'));
    }

    #[test]
    fn a_vector_crypto_group_stands_for_its_members() {
        let isa = parsed("rv64gcv_zvkng");

        for ext in [
            Extension::Zvkned,
            Extension::Zvknhb,
            Extension::Zvkb,
            Extension::Zvkt,
            Extension::Zvkg,
        ] {
            assert!(isa.extensions.contains(ext), "{ext:?}");
        }
        assert!(!isa.extensions.contains(Extension::Zvbc));
    }

    #[test]
    fn everything_is_what_the_full_string_parses_to() {
        let everything = Isa::everything(true);
        let names: Vec<String> =
            everything.extensions.iter().map(|ext| ext.name().to_owned()).collect();
        let text = format!("rv64imafdcbv_{}", names.join("_"));

        assert_eq!(parsed(&text), everything);
        assert_eq!(Isa::parse("rv64gcv_zvl256b").expect("valid").vlen, Some(256));
    }

    #[test]
    fn an_unknown_name_or_a_missing_base_is_refused() {
        assert!(
            matches!(Isa::parse("rv64gc_zxyz"), Err(IsaStringError::UnknownExtension(_, name)) if name == "zxyz")
        );
        assert!(
            matches!(Isa::parse("rv64imac_zfh"), Err(IsaStringError::Requires(_, name, 'f')) if name == "zfh")
        );
        assert!(matches!(Isa::parse("rv64gc_zvbb"), Err(IsaStringError::Requires(_, _, 'v'))));
        assert!(matches!(Isa::parse("rv64gcv_zvlxb"), Err(IsaStringError::MalformedVlen(..))));
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
