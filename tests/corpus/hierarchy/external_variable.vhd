-- difftest: top=external_variable std=2008 ref=hand
-- ref=hand: the reference simulator rejects method calls through external variable names
-- ("no method named ADD"); the expected output is the LRM result, which another reference
-- simulator prints.
-- External variable names of shared variables of a protected type (LRM 8.7): a testbench
-- reaches a scoreboard inside the design, directly and through an alias, from processes
-- elaborated before and after it; regions refer to each other's variables.
package sb_pkg is
  type counter is protected
    procedure add(n : integer);
    impure function total return integer;
  end protected;
end package;
package body sb_pkg is
  type counter is protected body
    variable sum : integer := 0;
    procedure add(n : integer) is
    begin
      sum := sum + n;
    end procedure;
    impure function total return integer is
    begin
      return sum;
    end function;
  end protected body;
end package body;
use work.sb_pkg.all;
entity dut is
  generic (step : integer := 1);
end;
architecture a of dut is
  shared variable sb : counter;
begin
  process
  begin
    for i in 1 to 3 loop
      sb.add(step);
      wait for 10 ns;
    end loop;
    -- The testbench's own scoreboard, from inside the design.
    <<variable .external_variable.seen : counter>>.add(100);
    wait;
  end process;
end;
use work.sb_pkg.all;
entity external_variable is end;
architecture tb of external_variable is
  shared variable seen : counter;
begin
  early : process
  begin
    wait for 5 ns;
    <<variable .external_variable.u.sb : counter>>.add(1000);
    report "early total=" & integer'image(<<variable .external_variable.u.sb : counter>>.total);
    wait;
  end process;
  u : entity work.dut generic map (step => 7);
  late : process
    alias sb is <<variable .external_variable.u.sb : counter>>;
  begin
    wait for 35 ns;
    report "late total=" & integer'image(sb.total) & " seen=" & integer'image(seen.total);
    assert sb.total = 1021 severity failure;
    assert seen.total = 100 severity failure;
    wait;
  end process;
end;
