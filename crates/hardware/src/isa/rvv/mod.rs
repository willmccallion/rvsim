//! Vector extension (RVV 1.0) vocabulary.
//!
//! Every domain-specific value in the vector pipeline uses a strict newtype
//! with no implicit conversions, so SEW bytes cannot be confused with SEW
//! bits, element indices with byte offsets, or vector register indices with
//! scalar ones.

mod group;
mod length;
mod vtype;

pub use group::{Eew, ElemIdx, Emul, Nf, VRegIdx};
pub use length::{Vl, Vlen, Vlmax};
pub use vtype::{
    LmulGroup, MaskPolicy, Sew, TailPolicy, VectorConfig, Vlmul, VtypeFields, Vxrm, encode_vtype,
    parse_vtype, parse_vtype_with_elen,
};
