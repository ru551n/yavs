-- difftest: top=lookalike_names std=2008
-- A type of the design named std_ulogic and a package of the design named std_logic_1164 are
-- the design's own, not the library's (#1163): their operators and edge functions run, and a
-- waveform on a signal of such a type works.
package my_logic is
  type std_ulogic is ('U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-');
  function "not"(x : std_ulogic) return std_ulogic;
  function "and"(a, b : std_ulogic) return std_ulogic;
end package;

package body my_logic is
  function "not"(x : std_ulogic) return std_ulogic is begin return x; end function;
  function "and"(a, b : std_ulogic) return std_ulogic is begin return '1'; end function;
end package body;

-- Not the library's package, though it is named so: operators and edge functions of the
-- library's std_ulogic and of bit, each unlike the library's.
library ieee;
use ieee.std_logic_1164.std_ulogic;

package std_logic_1164 is
  function "not"(x : std_ulogic) return std_ulogic;
  function "and"(a, b : std_ulogic) return std_ulogic;
  function "??"(x : std_ulogic) return boolean;
  function rising_edge(signal s : std_ulogic) return boolean;
  function rising_edge(signal s : bit) return boolean;
end package;

package body std_logic_1164 is
  function "not"(x : std_ulogic) return std_ulogic is
  begin
    return x;
  end function;
  function "and"(a, b : std_ulogic) return std_ulogic is
  begin
    return '1';
  end function;
  function "??"(x : std_ulogic) return boolean is
  begin
    return x = '0';
  end function;
  function rising_edge(signal s : std_ulogic) return boolean is
  begin
    return s'event;
  end function;
  function rising_edge(signal s : bit) return boolean is
  begin
    return s'event;
  end function;
end package body;

use work.my_logic.all;

entity lookalike_type is
end entity;

architecture a of lookalike_type is
  signal x, y, z, w : std_ulogic := '0';
begin
  y <= not x;
  z <= x and '0';
  w <= '1' after 5 ns, '0' after 10 ns;
  process begin
    wait for 20 ns;
    report "y=" & std_ulogic'image(y) & " z=" & std_ulogic'image(z) & " w=" & std_ulogic'image(w);
    assert y = '0' and z = '1' and w = '0' report "type" severity failure;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.std_ulogic;

entity lookalike_package is
end entity;

architecture a of lookalike_package is
  signal x, y, z, clk, rst : std_ulogic := '0';
  signal bclk : bit := '0';
  signal cnt, bcnt, q : integer := 0;
begin
  y <= work.std_logic_1164."not"(x);
  z <= work.std_logic_1164."and"(x, x);
  clk <= '1' after 5 ns, '0' after 10 ns;
  bclk <= '1' after 5 ns, '0' after 10 ns;
  process (clk) begin
    if work.std_logic_1164.rising_edge(clk) then
      cnt <= cnt + 1;
    end if;
  end process;
  process (bclk) begin
    if work.std_logic_1164.rising_edge(bclk) then
      bcnt <= bcnt + 1;
    end if;
  end process;
  -- The `??` of the package: true while rst is '0'.
  process (clk, rst) begin
    if work.std_logic_1164."??"(rst) then
      q <= 100;
    elsif clk'event and clk = '1' then
      q <= q + 1;
    end if;
  end process;
  process begin
    wait for 30 ns;
    report "y=" & std_ulogic'image(y) & " z=" & std_ulogic'image(z) & " cnt=" & integer'image(cnt)
      & " bcnt=" & integer'image(bcnt) & " q=" & integer'image(q);
    assert y = '0' and z = '1' and cnt = 2 and bcnt = 2 and q = 100 report "package" severity failure;
    wait;
  end process;
end architecture;

entity lookalike_names is
end entity;

architecture tb of lookalike_names is
begin
  t : entity work.lookalike_type;
  p : entity work.lookalike_package;
end architecture;
