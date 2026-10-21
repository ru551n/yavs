//! With variables writable from outside (`opt::external_variables`) a variable is unknown
//! after a suspension. A process of its own: the setting is global.

mod proc_common;
use proc_common::sums_left;

#[test]
fn a_deposited_variable_is_unknown_after_a_wait() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    yavs_ir::opt::external_variables();
    assert_eq!(sums_left(6, true), 1);
    assert_eq!(sums_left(4, false), 1);
}
