//! Timing tables resolve JESD79-5B constraints for the speed bin.

use rvsim_core::soc::memory::ddr5::{Constraint, Ddr5SpeedBin, Ddr5Timing};

#[test]
fn ddr5_4800b_resolves_to_jedec_clock_counts() {
    let t = Ddr5Timing::default();
    assert_eq!(t.data_rate_mts, 4800);
    assert_eq!(t.t_cas, 40);
    assert_eq!(t.t_cwl, 38);
    assert_eq!(t.t_rcd, 39);
    assert_eq!(t.t_rp, 39);
    assert_eq!(t.t_ras, 77);
    assert_eq!(t.t_rc, 116);
    assert_eq!(t.t_rrd_s, 8);
    assert_eq!(t.t_rrd_l, 12);
    assert_eq!(t.t_ccd_s, 8);
    assert_eq!(t.t_ccd_l, 12);
    assert_eq!(t.t_ccd_l_wr, 48);
    assert_eq!(t.t_faw, 32);
    assert_eq!(t.t_wtr_s, 6);
    assert_eq!(t.t_wtr_l, 24);
    assert_eq!(t.t_rtw, 2);
    assert_eq!(t.t_wr, 72);
    assert_eq!(t.t_rtp, 18);
    assert_eq!(t.t_ppd, 2);
    assert_eq!(t.t_rtrs, 2);
    assert_eq!(t.t_rfc1, 708);
    assert_eq!(t.t_rfc2, 384);
    assert_eq!(t.t_rfcsb, 312);
    assert_eq!(t.t_refi, 9360);
    assert_eq!(t.t_xp, 18);
    assert_eq!(t.t_pd, 18);
    assert_eq!(t.bl_half, 8);
}

#[test]
fn t_rc_equals_t_ras_plus_t_rp() {
    let t = Ddr5Timing::default();
    assert_eq!(t.t_rc, t.t_ras + t.t_rp);
}

#[test]
fn tck_is_two_over_the_data_rate() {
    let t = Ddr5Timing::default();
    assert_eq!(t.tck_ps(), 416);
}

#[test]
fn constraint_takes_the_larger_of_clocks_and_rounded_up_time() {
    let c = Constraint::max(8, 5_000);
    assert_eq!(c.cycles(4800), 12, "5 ns at 2400 MHz is exactly 12 clocks");
    assert_eq!(c.cycles(3200), 8, "5 ns at 1600 MHz is 8 clocks, tied with the floor");
    assert_eq!(Constraint::ps(32_000).cycles(4800), 77, "32 ns rounds 76.8 up");
    assert_eq!(Constraint::clocks(32).cycles(4800), 32);
}

#[test]
fn ddr5_5600b_scales_time_constraints_with_the_faster_clock() {
    let t = Ddr5Timing::from_bin(&Ddr5SpeedBin::DDR5_5600B);
    assert_eq!(t.t_cas, 46);
    assert_eq!(t.t_cwl, 44);
    assert_eq!(t.t_rcd, 45);
    assert_eq!(t.t_rp, 45);
    assert_eq!(t.t_ras, 90);
    assert_eq!(t.t_rrd_l, 14);
    assert_eq!(t.t_faw, 38);
    assert_eq!(t.t_refi, 10920);
    assert_eq!(t.t_rfc1, 826);
}
