//! SHA-256 message schedule and compression (Zvknha/Zvknhb).

#[inline]
pub(super) const fn sha256_sigma0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}

#[inline]
pub(super) const fn sha256_sigma1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}

#[inline]
pub(super) const fn sha256_sum0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}

#[inline]
pub(super) const fn sha256_sum1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}

#[inline]
pub(super) const fn sha256_ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}

#[inline]
pub(super) const fn sha256_maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}

/// vsha2ms.vv — SHA-256 message scheduling. Per Zvknha:
///   vd  = {W3,  W2,  W1,  W0 } (vd[0]=W0, vd[3]=W3)
///   vs2 = {W11, W10, W9,  W4 } (vs2[0]=W4, vs2[3]=W11)
///   vs1 = {W15, W14, W13, W12} (vs1[0]=W12, vs1[3]=W15)
/// Output replaces vd with {W19, W18, W17, W16}.
pub(super) const fn sha256_ms(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4]) -> [u32; 4] {
    let w0 = vd[0];
    let w1 = vd[1];
    let w2 = vd[2];
    let w3 = vd[3];
    let w4 = vs2[0];
    let w9 = vs2[1];
    let w10 = vs2[2];
    let w11 = vs2[3];
    let w12 = vs1[0];
    let w14 = vs1[2];
    let w15 = vs1[3];

    let w16 = sha256_sigma1(w14).wrapping_add(w9).wrapping_add(sha256_sigma0(w1)).wrapping_add(w0);
    let w17 = sha256_sigma1(w15).wrapping_add(w10).wrapping_add(sha256_sigma0(w2)).wrapping_add(w1);
    let w18 = sha256_sigma1(w16).wrapping_add(w11).wrapping_add(sha256_sigma0(w3)).wrapping_add(w2);
    let w19 = sha256_sigma1(w17).wrapping_add(w12).wrapping_add(sha256_sigma0(w4)).wrapping_add(w3);
    [w16, w17, w18, w19]
}

/// One SHA-256 round step on the (a..h) state, mutating in place per FIPS-180-4.
#[inline]
pub(super) const fn sha256_round(state: &mut [u32; 8], kw: u32) {
    let [a, b, c, d, e, f, g, h] = *state;
    let t1 = h.wrapping_add(sha256_sum1(e)).wrapping_add(sha256_ch(e, f, g)).wrapping_add(kw);
    let t2 = sha256_sum0(a).wrapping_add(sha256_maj(a, b, c));
    *state = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
}

/// vsha2cl.vv / vsha2ch.vv share state layout per Zvknha:
///   vd  = {c, d, g, h}     (vd[0]=h, vd[1]=g, vd[2]=d, vd[3]=c)
///   vs2 = {a, b, e, f}     (vs2[0]=f, vs2[1]=e, vs2[2]=b, vs2[3]=a)
///   vs1 = {kw3, kw2, kw1, kw0}
/// .vsha2cl runs 2 compression rounds with kw0 then kw1; .vsha2ch uses
/// kw2 then kw3. After two rounds the destination is rewritten to the
/// updated low half {a', b', e', f'}.
pub(super) fn sha256_compress(
    vd: [u32; 4],
    vs2: [u32; 4],
    vs1: [u32; 4],
    kw_indices: [usize; 2],
) -> [u32; 4] {
    let h = vd[0];
    let g = vd[1];
    let d = vd[2];
    let c = vd[3];
    let f = vs2[0];
    let e = vs2[1];
    let b = vs2[2];
    let a = vs2[3];

    let mut state = [a, b, c, d, e, f, g, h];
    for &idx in &kw_indices {
        sha256_round(&mut state, vs1[idx]);
    }
    [state[5], state[4], state[1], state[0]]
}

#[inline]
pub(super) fn sha256_compress_low(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4]) -> [u32; 4] {
    sha256_compress(vd, vs2, vs1, [0, 1])
}

#[inline]
pub(super) fn sha256_compress_high(vd: [u32; 4], vs2: [u32; 4], vs1: [u32; 4]) -> [u32; 4] {
    sha256_compress(vd, vs2, vs1, [2, 3])
}
