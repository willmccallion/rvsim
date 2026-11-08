//! The timed latches between pipeline stages.

use crate::uarch::pipeline::latches::Latch;

#[test]
fn entries_put_back_are_taken_before_those_written_since() {
    let mut latch = Latch::new(1);
    latch.push(0, [1, 2, 3]);
    let mut taken = latch.take(1);
    let refused = taken.split_off(1);

    latch.put_back(1, refused);
    latch.push(1, [4]);

    assert_eq!(latch.take(2), [2, 3]);
    assert_eq!(latch.take(2), [4]);
}

#[test]
fn putting_back_nothing_leaves_the_latch_empty() {
    let mut latch = Latch::<u32>::new(1);

    latch.put_back(0, Vec::new());

    assert!(latch.is_empty());
}
