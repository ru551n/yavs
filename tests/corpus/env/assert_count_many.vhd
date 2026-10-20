-- difftest: top=assert_count_many std=2019
-- Many reports in one cycle, interleaved with enable, clear and severities: the counts stay exact
-- (the per-process change log of the assert state folds consecutive counts).
use std.env.all;
entity assert_count_many is end entity;

architecture test of assert_count_many is
begin
  process
  begin
    for i in 1 to 3000 loop
      if i mod 2 = 0 then
        assert false report "w" severity warning;
      else
        assert false report "n" severity note;
      end if;
      if i mod 1000 = 0 then
        SetVhdlAssertEnable(note, i mod 2000 = 0);
      end if;
    end loop;
    SetVhdlAssertEnable(note, true);
    report "w " & integer'image(GetVhdlAssertCount(warning)) & " n " & integer'image(GetVhdlAssertCount(note));
    ClearVhdlAssert;
    for i in 1 to 10 loop
      assert false report "x" severity warning;
    end loop;
    report "after " & integer'image(GetVhdlAssertCount);
    wait;
  end process;
end architecture;
