//! Memory ordering / FENCE Unit Tests.
//!
//! Verifies FENCE instruction decoding, predecessor/successor
//! ordering set extraction, TSO recognition, and no-op detection.

use riscv_core::core::units::lsu::ordering::{Fence, FenceSet};

// ══════════════════════════════════════════════════════════
// 1. FenceSet basics
// ══════════════════════════════════════════════════════════

#[test]
fn fence_set_from_bits_all_zero() {
    let fs = FenceSet::from_bits(0b0000);
    assert!(!fs.i);
    assert!(!fs.o);
    assert!(!fs.r);
    assert!(!fs.w);
    assert!(fs.is_empty());
    assert!(!fs.is_full());
}

#[test]
fn fence_set_from_bits_all_ones() {
    let fs = FenceSet::from_bits(0b1111);
    assert!(fs.i);
    assert!(fs.o);
    assert!(fs.r);
    assert!(fs.w);
    assert!(!fs.is_empty());
    assert!(fs.is_full());
}

#[test]
fn fence_set_from_bits_rw() {
    let fs = FenceSet::from_bits(0b0011);
    assert!(!fs.i);
    assert!(!fs.o);
    assert!(fs.r);
    assert!(fs.w);
}

#[test]
fn fence_set_from_bits_io() {
    let fs = FenceSet::from_bits(0b1100);
    assert!(fs.i);
    assert!(fs.o);
    assert!(!fs.r);
    assert!(!fs.w);
}

#[test]
fn fence_set_round_trip() {
    for bits in 0..=0xF_u8 {
        let fs = FenceSet::from_bits(bits);
        assert_eq!(
            fs.to_bits(),
            bits,
            "Round-trip failed for bits {:#06b}",
            bits
        );
    }
}

// ══════════════════════════════════════════════════════════
// 2. Fence decode: IORW,IORW (full barrier)
// ══════════════════════════════════════════════════════════

#[test]
fn fence_full_barrier_iorw_iorw() {
    // FENCE IORW, IORW encoding: opcode=0x0F, pred=0b1111 (bits 27:24), succ=0b1111 (bits 23:20)
    let inst: u32 = 0x0FF0_000F; // pred=0xF, succ=0xF
    let fence = Fence::decode(inst);

    assert!(fence.pred.is_full());
    assert!(fence.succ.is_full());
    assert!(fence.is_full_barrier());
    assert!(!fence.is_nop());
    assert!(!fence.is_tso());
}

// ══════════════════════════════════════════════════════════
// 3. Fence decode: RW,RW (TSO-like)
// ══════════════════════════════════════════════════════════

#[test]
fn fence_rw_rw_is_tso() {
    // pred=0b0011 (RW), succ=0b0011 (RW)
    let inst: u32 = (0b0011 << 24) | (0b0011 << 20) | 0x0F;
    let fence = Fence::decode(inst);

    assert!(fence.pred.r);
    assert!(fence.pred.w);
    assert!(!fence.pred.i);
    assert!(!fence.pred.o);
    assert!(fence.succ.r);
    assert!(fence.succ.w);
    assert!(fence.is_tso());
    assert!(!fence.is_full_barrier());
}

// ══════════════════════════════════════════════════════════
// 4. Fence decode: no bits (nop)
// ══════════════════════════════════════════════════════════

#[test]
fn fence_no_bits_is_nop() {
    let inst: u32 = 0x0000_000F; // pred=0, succ=0
    let fence = Fence::decode(inst);

    assert!(fence.pred.is_empty());
    assert!(fence.succ.is_empty());
    assert!(fence.is_nop());
    assert!(!fence.is_full_barrier());
}

// ══════════════════════════════════════════════════════════
// 5. Fence decode: W,R (store-load barrier)
// ══════════════════════════════════════════════════════════

#[test]
fn fence_w_r_store_load_barrier() {
    // pred=0b0001 (W), succ=0b0010 (R)
    let inst: u32 = (0b0001 << 24) | (0b0010 << 20) | 0x0F;
    let fence = Fence::decode(inst);

    assert!(!fence.pred.i);
    assert!(!fence.pred.o);
    assert!(!fence.pred.r);
    assert!(fence.pred.w);
    assert!(!fence.succ.i);
    assert!(!fence.succ.o);
    assert!(fence.succ.r);
    assert!(!fence.succ.w);
    assert!(!fence.is_tso());
    assert!(!fence.is_nop());
}

// ══════════════════════════════════════════════════════════
// 6. Asymmetric pred/succ
// ══════════════════════════════════════════════════════════

#[test]
fn fence_asymmetric_pred_succ() {
    // pred=IORW (0xF), succ=R (0b0010)
    let inst: u32 = (0b1111 << 24) | (0b0010 << 20) | 0x0F;
    let fence = Fence::decode(inst);

    assert!(fence.pred.is_full());
    assert!(!fence.succ.is_full());
    assert!(fence.succ.r);
    assert!(!fence.succ.w);
    assert!(!fence.is_full_barrier());
}

// ══════════════════════════════════════════════════════════
// 7. FenceSet individual bit flags
// ══════════════════════════════════════════════════════════

#[test]
fn fence_set_only_i() {
    let fs = FenceSet::from_bits(0b1000);
    assert!(fs.i);
    assert!(!fs.o);
    assert!(!fs.r);
    assert!(!fs.w);
    assert!(!fs.is_empty());
    assert!(!fs.is_full());
}

#[test]
fn fence_set_only_o() {
    let fs = FenceSet::from_bits(0b0100);
    assert!(!fs.i);
    assert!(fs.o);
    assert!(!fs.r);
    assert!(!fs.w);
}

#[test]
fn fence_set_only_r() {
    let fs = FenceSet::from_bits(0b0010);
    assert!(!fs.i);
    assert!(!fs.o);
    assert!(fs.r);
    assert!(!fs.w);
}

#[test]
fn fence_set_only_w() {
    let fs = FenceSet::from_bits(0b0001);
    assert!(!fs.i);
    assert!(!fs.o);
    assert!(!fs.r);
    assert!(fs.w);
}
