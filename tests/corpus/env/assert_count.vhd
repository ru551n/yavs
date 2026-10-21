-- difftest: top=assert_count std=2019
-- VHDL assert state of std.env (LRM 2019 16.5): counts per severity (report statements
-- count too, as sim1), IsVhdlAssertFailed, SetVhdlAssertEnable hides and stops counting a
-- severity, ClearVhdlAssert, the default format and read severity. The exit status after
-- ClearVhdlAssert is env/assert_clear_exit.
use std.env.all;
entity assert_count is end entity;

architecture test of assert_count is
begin
  process
  begin
    assert false report "a1" severity warning;
    assert false report "a2" severity warning;
    report "r1" severity note;
    report "count " & integer'image(GetVhdlAssertCount) & ", warnings "
      & integer'image(GetVhdlAssertCount(warning)) & ", notes "
      & integer'image(GetVhdlAssertCount(note));
    assert IsVhdlAssertFailed and IsVhdlAssertFailed(warning) and IsVhdlAssertFailed(note)
      severity failure;
    assert not IsVhdlAssertFailed(error) and not IsVhdlAssertFailed(failure) severity failure;
    SetVhdlAssertEnable(warning, false);
    assert false report "hidden" severity warning;
    assert not GetVhdlAssertEnable(warning) and GetVhdlAssertEnable(error) severity failure;
    assert GetVhdlAssertCount(warning) = 2 severity failure;
    SetVhdlAssertEnable(true);
    ClearVhdlAssert;
    report "after clear " & integer'image(GetVhdlAssertCount);
    report "format [" & GetVhdlAssertFormat(error) & "], read severity "
      & severity_level'image(GetVhdlReadSeverity);
    SetVhdlReadSeverity(warning);
    assert GetVhdlReadSeverity = warning severity failure;
    wait;
  end process;
end architecture;
