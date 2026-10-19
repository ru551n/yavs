-- Coverage counts (crates/yavs/tests/coverage.rs): every count below is pinned there.
package cov_pkg is
  function sign(x : integer) return integer;
end package;

package body cov_pkg is
  function sign(x : integer) return integer is
  begin
    if x > 0 then
      return 1;
    elsif x < 0 then
      return -1;
    end if;
    return 0;
  end function;
end package body;

entity cov_sub is
  port (a : in integer; y : out integer);
end entity;

architecture rtl of cov_sub is
begin
  decode : process (a)
  begin
    case a is
      when 0 => y <= 10;
      when 1 => y <= 11;
      when others => y <= 12;
    end case;
  end process;
end architecture;

library ieee;
use ieee.numeric_std.all;
use work.cov_pkg.all;

entity cov_run is
  generic (n : integer := 3);
end entity;

architecture sim of cov_run is
  signal a, b, y1, y2, s : integer := 0;
begin
  u1 : entity work.cov_sub port map (a => a, y => y1);
  u2 : entity work.cov_sub port map (a => b, y => y2);
  s <= 1 when a = 5 else 2;
  stim : process
    variable v : integer := 0;
  begin
    for i in 1 to n loop
      v := v + sign(i);
    end loop;
    assert to_integer(to_unsigned(v, 8)) = n;
    while v < 0 loop
      v := 0;
    end loop;
    if v = n then
      a <= 1;
    else
      a <= 2;
    end if;
    wait for 1 ns;
    a <= 7;
    wait;
  end process;
end architecture;
