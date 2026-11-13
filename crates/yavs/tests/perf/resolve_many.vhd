-- Resolved signals with many drivers that all assign every clock: two std_logic signals of 16
-- drivers each ('1' or 'Z'), and an integer with a resolution function of 12 drivers. Each
-- resolved element is resolved once per update from all its sources (`resolutions`), not once
-- per driver whose transaction came in.
library ieee;
use ieee.std_logic_1164.all;

package resolve_many_pkg is
  type int_v is array (natural range <>) of integer;
  function res_max (v : int_v) return integer;
  subtype rint is res_max integer;
end;

package body resolve_many_pkg is
  function res_max (v : int_v) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop
      if v(i) > m then m := v(i); end if;
    end loop;
    return m;
  end;
end;

library ieee;
use ieee.std_logic_1164.all;
use work.resolve_many_pkg.all;

entity resolve_many is end;
architecture a of resolve_many is
  type sa is array (0 to 1) of std_logic;
  signal clk : std_logic := '0';
  signal sl : sa;
  signal ri : rint := 0;
  signal sum : integer := 0;
begin
  clkgen : process
  begin
    for i in 1 to 50 loop
      clk <= '1';
      wait for 1 ns;
      clk <= '0';
      wait for 1 ns;
    end loop;
    wait for 1 ns;
    report "sum=" & integer'image(sum);
    wait;
  end process;

  gs : for i in 0 to 1 generate
    gk : for k in 0 to 15 generate
      process (clk)
        variable x : natural := i * 31 + k * 7 + 1;
      begin
        if rising_edge(clk) then
          x := (x * 1103 + 12345) mod 1000003;
          if x mod 5 = 0 then sl(i) <= '1'; else sl(i) <= 'Z'; end if;
        end if;
      end process;
    end generate;
  end generate;

  gi : for k in 0 to 11 generate
    process (clk)
      variable x : natural := k * 7 + 1;
    begin
      if rising_edge(clk) then
        x := (x * 1103 + 12345) mod 1000003;
        ri <= x mod 1000;
      end if;
    end process;
  end generate;

  acc : process (clk)
    variable a : integer := 0;
  begin
    if falling_edge(clk) then
      if sl(0) = '1' then a := a + 1; end if;
      if sl(1) = '1' then a := a + 2; end if;
      a := (a + ri) mod 100003;
      sum <= a;
    end if;
  end process;
end;
