-- difftest: top=external_subprogram_variable std=2008 ref=hand
-- ref=hand: the reference simulator rejects method calls through external variable names
-- ("no method named ADD"); the expected output is the LRM result, which another reference
-- simulator prints.
-- External shared variables of a protected type named in subprograms (LRM 8.7, #255): a
-- package procedure updates the testbench's scoreboard through an absolute path and a
-- package's scoreboard through a package path (`<<variable @work.pkg.v : t>>`), a process
-- calls methods through a package path, and a method of a protected type declared in a
-- package samples a design signal through an external name.
package sb_pkg is
  type sb_t is protected
    procedure add(n : integer);
    impure function total return integer;
  end protected;
end package;
package body sb_pkg is
  type sb_t is protected body
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
package global_pkg is
  shared variable events : sb_t;
end package;

use work.sb_pkg.all;
package ops_pkg is
  procedure record_hit(n : integer);
  impure function hits return integer;
  type monitor_t is protected
    procedure sample;
    impure function seen return integer;
  end protected;
end package;
package body ops_pkg is
  procedure record_hit(n : integer) is
  begin
    <<variable .external_subprogram_variable.sb : sb_t>>.add(n);
    <<variable @work.global_pkg.events : sb_t>>.add(1);
  end procedure;
  impure function hits return integer is
    alias s is <<variable .external_subprogram_variable.sb : sb_t>>;
  begin
    return s.total;
  end function;
  type monitor_t is protected body
    variable ones : integer := 0;
    procedure sample is
    begin
      if <<signal .external_subprogram_variable.dut.s : bit>> = '1' then
        ones := ones + 1;
      end if;
    end procedure;
    impure function seen return integer is
    begin
      return ones;
    end function;
  end protected body;
end package body;

entity dut is end;
architecture a of dut is
  signal s : bit := '0';
begin
  s <= '1' after 1 ns, '0' after 2 ns, '1' after 3 ns;
end;

use work.sb_pkg.all;
use work.ops_pkg.all;
entity external_subprogram_variable is end;
architecture tb of external_subprogram_variable is
  shared variable sb : sb_t;
  shared variable mon : monitor_t;
begin
  dut : entity work.dut;
  process
  begin
    for i in 1 to 4 loop
      record_hit(i * 10);
      mon.sample;
      wait for 1 ns;
    end loop;
    report "hits=" & integer'image(hits) & " events="
      & integer'image(<<variable @work.global_pkg.events : sb_t>>.total)
      & " ones=" & integer'image(mon.seen);
    assert hits = 100 and sb.total = 100 severity failure;
    assert mon.seen = 2 severity failure;
    wait;
  end process;
end;
