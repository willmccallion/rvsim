//! What memory instructions compute from the data they access.

use crate::exec::compute::fpu::half::box_f16;
use crate::exec::compute::fpu::nan_handling::F32_NAN_BOX_MASK;
use crate::isa::bits::zext_word;
use crate::isa::op::MemWidth;

/// The value a load writes to its destination: `raw` sign- or
/// zero-extended to its width, and NaN-boxed when it is a narrower
/// floating-point value.
pub const fn load_result(raw: u64, width: MemWidth, signed: bool, fp_dest: bool) -> u64 {
    let value = extend_load(raw, width, signed);
    if !fp_dest {
        return value;
    }
    match width {
        MemWidth::Word => value | F32_NAN_BOX_MASK,
        MemWidth::Half => box_f16(value as u16),
        _ => value,
    }
}

/// Sign / zero-extends a raw load value according to the access width and
/// the signed-load control bit.
const fn extend_load(raw: u64, width: MemWidth, signed: bool) -> u64 {
    if signed {
        match width {
            MemWidth::Byte => (raw as u8 as i8) as i64 as u64,
            MemWidth::Half => (raw as u16 as i16) as i64 as u64,
            MemWidth::Word => (raw as u32 as i32) as i64 as u64,
            MemWidth::Double => raw,
            MemWidth::Nop => 0,
        }
    } else {
        match width {
            MemWidth::Byte => raw & 0xFF,
            MemWidth::Half => raw & 0xFFFF,
            MemWidth::Word => zext_word(raw),
            MemWidth::Double => raw,
            MemWidth::Nop => 0,
        }
    }
}
