//! The predictors' random replacement draws from Marsaglia's 64-bit
//! xorshift generator with shifts 13, 7 and 17 ("Xorshift RNGs", 2003).

use crate::uarch::bpred::components::xorshift64;

#[test]
fn xorshift64_from_one_gives_the_13_7_17_sequence() {
    let mut state = 1;

    let draws = [xorshift64(&mut state), xorshift64(&mut state), xorshift64(&mut state)];

    assert_eq!(draws, [0x4082_2041, 0x1000_4106_0c01_1441, 0x9b1e_842f_6e86_2629]);
    assert_eq!(state, draws[2], "the state is the last draw");
}
