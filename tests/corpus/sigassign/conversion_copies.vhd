-- difftest: top=tb_conversion_copies std=2008
-- Concurrent assignments that only copy a signal through a conversion between arrays of one
-- element type (`unsigned(v)`, `std_logic_vector(u(5 downto 2))`), whole, sliced and into a
-- part of the target: values, events and deltas must be those of the equivalent processes.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_conversion_copies is end;

architecture t of tb_conversion_copies is
  signal v  : std_logic_vector(7 downto 0);
  signal u  : unsigned(7 downto 0);
  signal w  : std_logic_vector(3 downto 0);
  signal s  : signed(0 to 3);
  signal p  : std_logic_vector(7 downto 0) := (others => '-');
  signal n  : natural := 0;
begin
  u <= unsigned(v);
  w <= std_logic_vector(u(5 downto 2));
  s <= signed(v(3 downto 0));
  p(7 downto 4) <= std_logic_vector(u(3 downto 0));
  p(3 downto 0) <= std_logic_vector(s);

  watch : process (u, w, s, p)
  begin
    n <= n + 1;
    report "u=" & to_string(u) & " w=" & to_string(w) & " s=" & to_string(s) & " p=" & to_string(p)
      & " ev=" & boolean'image(u'event) & boolean'image(w'event) & boolean'image(s'event)
      & boolean'image(p'event);
  end process;

  stim : process
  begin
    wait for 1 ns;
    v <= x"a5";
    wait for 1 ns;
    -- the same value again: a transaction, no event
    v <= x"a5";
    wait for 1 ns;
    v <= "0011XZ01";
    wait for 1 ns;
    v <= x"3c";
    wait for 1 ns;
    assert u = unsigned'(x"3c") report "u" severity failure;
    assert w = "1111" report "w" severity failure;
    assert s = signed'("1100") report "s" severity failure;
    assert p = x"cc" report "p" severity failure;
    wait;
  end process;
end architecture;
