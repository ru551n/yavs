-- difftest: top=assert_clear_exit std=2019
-- The exit status follows the VHDL assert counts of std.env at the end of the run (LRM 2019
-- 16.5, as the reference simulator): an error report followed by ClearVhdlAssert exits 0 (#191).
use std.env.all;
entity assert_clear_exit is end entity;

architecture test of assert_clear_exit is
begin
  process
  begin
    report "expected error" severity error;
    assert GetVhdlAssertCount(error) = 1 severity failure;
    ClearVhdlAssert;
    report "cleared " & integer'image(GetVhdlAssertCount);
    wait;
  end process;
end architecture;
