-- The analysis cache of cycle mode and an elaboration that reads a file: a register of the region
-- adds a step that a constant reads from `acfile.txt` with an impure function. With `WR` the
-- elaboration also writes a file.
library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;

entity acfile is
  generic (WR : boolean := false);
end entity;

architecture tb of acfile is
  impure function from_file return natural is
    file f : text open read_mode is "acfile.txt";
    variable l : line;
    variable v : natural;
  begin
    readline(f, l);
    read(l, v);
    return v;
  end function;
  impure function maybe_write(w : boolean) return natural is
    file f : text;
    variable l : line;
  begin
    if w then
      file_open(f, "acfile.out", write_mode);
      write(l, string'("x"));
      writeline(f, l);
      file_close(f);
    end if;
    return 0;
  end function;
  constant STEP : natural := from_file + maybe_write(WR);
  signal clk : std_ulogic := '0';
  signal a : natural range 0 to 999 := 0;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      a <= (a + STEP) mod 1000;
    end if;
  end process;
  main : process
  begin
    for i in 1 to 10 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "a=" & integer'image(a);
    wait;
  end process;
end architecture;
