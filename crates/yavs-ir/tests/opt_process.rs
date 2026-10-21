//! Interval analysis in a process: facts hold within one activation, and across a suspension
//! only for a variable whose every store (and zero start) stays inside them.

mod proc_common;
use proc_common::sums_left;

#[test]
fn a_variable_keeps_its_range_across_a_wait() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    // The variable is 0, 3 or 5: plus one is at most 6.
    assert_eq!(sums_left(6, true), 0);
    assert_eq!(sums_left(5, true), 1);
}

#[test]
fn nothing_follows_a_wait_for_ever() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    // Only the first activation reaches the sum, and the variable is 3 there.
    assert_eq!(sums_left(4, false), 0);
    assert_eq!(sums_left(3, false), 1);
}
