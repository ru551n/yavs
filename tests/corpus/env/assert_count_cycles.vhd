-- difftest: top=assert_count_cycles std=2019 ref=hand
-- The assert state is global but read deterministically (docs/decisions.md, #99): a process
-- sees the counts as of the start of its cycle plus its own reports, so a report of another
-- process in the same cycle shows up one delta later, whatever order the processes run in.
-- ref=hand: sim1 1.23 counts at once, so b sees 1 or 0 depending on the process order.
use std.env.all;
entity assert_count_cycles is end entity;

architecture test of assert_count_cycles is
begin
  a: process
  begin
    assert false report "from a" severity warning;
    report "a sees " & integer'image(GetVhdlAssertCount);
    wait;
  end process;

  b: process
  begin
    report "b sees " & integer'image(GetVhdlAssertCount(warning)) severity note;
    wait for 0 ns;
    report "b then sees " & integer'image(GetVhdlAssertCount(warning)) severity note;
    wait;
  end process;
end architecture;
