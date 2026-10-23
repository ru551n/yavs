-- difftest: top=textio_93 std=1993
-- The 1993 TEXTIO and the Synopsys STD_LOGIC_TEXTIO (the hex/octal routines are theirs).
library ieee;
use ieee.std_logic_1164.all;
use ieee.std_logic_textio.all;
use std.textio.all;

entity textio_93 is end;

architecture a of textio_93 is
begin
  process
    variable l : line;
    variable slv : std_logic_vector(7 downto 0) := x"A5";
    variable r : std_logic_vector(7 downto 0);
    variable o6 : std_logic_vector(5 downto 0) := "101110";
    variable bv : bit_vector(3 downto 0);
    variable n : integer;
    variable t : time;
    variable ok : boolean;
  begin
    write(l, string'("slv "));
    write(l, slv);
    write(l, ' ');
    hwrite(l, slv);
    write(l, ' ');
    owrite(l, o6);
    write(l, ' ');
    write(l, 42);
    write(l, ' ');
    write(l, 5 ns, right, 8);
    writeline(output, l);

    write(l, string'("1011 7f 1234 7 ns"));
    read(l, bv);
    hread(l, r(7 downto 0));
    read(l, n);
    read(l, t, ok);
    assert ok severity failure;
    write(l, string'("bv "));
    write(l, bv);
    write(l, string'(" r "));
    write(l, r);
    write(l, string'(" n "));
    write(l, n);
    write(l, string'(" t "));
    write(l, t);
    writeline(output, l);
    wait;
  end process;
end architecture;
