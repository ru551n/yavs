-- difftest: top=assert_format std=2019
-- SetVhdlAssertFormat stores a format per severity (#922). Invalid formats: crates/yavs/tests/assert_format.rs (the reference prints its own error)..
use std.env.all;
entity assert_format is end entity;
architecture a of assert_format is
begin
  process
    variable valid : boolean;
  begin
    SetVhdlAssertFormat(failure, "Wacky Failure");
    assert GetVhdlAssertFormat(failure) = "Wacky Failure" severity failure;
    assert GetVhdlAssertFormat(warning) = "" severity failure;
    SetVhdlAssertFormat(warning, "Wackier Warning", valid);
    assert valid severity failure;
    assert GetVhdlAssertFormat(warning) /= "" severity failure;
    SetVhdlAssertFormat(failure, "");
    assert GetVhdlAssertFormat(failure) = "" severity failure;
    report "done";
    wait;
  end process;
end architecture;
