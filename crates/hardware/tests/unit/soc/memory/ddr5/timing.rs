//! Asserts every field of `Ddr5Timing::default()` matches the JESD79-5B
//! DDR5-4800 speed bin.

use rvsim_core::soc::memory::ddr5::Ddr5Timing;

#[test]
fn defaults_match_jedec_ddr5_4800() {
    let t = Ddr5Timing::default();
    assert_eq!(t.t_cas, 40);
    assert_eq!(t.t_cwl, 38);
    assert_eq!(t.t_rcd, 40);
    assert_eq!(t.t_rp, 40);
    assert_eq!(t.t_ras, 77);
    assert_eq!(t.t_rc, 117);
    assert_eq!(t.t_rrd_s, 8);
    assert_eq!(t.t_rrd_l, 8);
    assert_eq!(t.t_ccd_s, 8);
    assert_eq!(t.t_ccd_l, 8);
    assert_eq!(t.t_ccd_l_wr, 32);
    assert_eq!(t.t_faw, 32);
    assert_eq!(t.t_wtr_s, 4);
    assert_eq!(t.t_wtr_l, 12);
    assert_eq!(t.t_rtw, 8);
    assert_eq!(t.t_wr, 30);
    assert_eq!(t.t_rtp, 12);
    assert_eq!(t.t_rtrs, 2);
    assert_eq!(t.t_rfc1, 984);
    assert_eq!(t.t_rfc2, 528);
    assert_eq!(t.t_refi, 3900);
    assert_eq!(t.bl_half, 8);
}

#[test]
fn t_rc_equals_t_ras_plus_t_rp() {
    let t = Ddr5Timing::default();
    assert_eq!(t.t_rc, t.t_ras + t.t_rp);
}
