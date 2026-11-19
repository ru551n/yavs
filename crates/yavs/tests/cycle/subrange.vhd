-- A subtype of an architecture whose range comes from a function elaboration runs and the region's
-- constant folding does not (it calls `numeric_std`), used as a slice range, a loop range and in
-- `'length` (`W` replaced by a width; instances of 1, W and W + 4 elements of 1 to 132).
library ieee;
use ieee.numeric_std.all;
package cycle_subrange_pkg is
  function width_of(n : natural) return positive;
end package;
package body cycle_subrange_pkg is
  function width_of(n : natural) return positive is
    constant v : unsigned(63 downto 0) := to_unsigned(n, 64);
  begin
    return to_integer(v);
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_subrange_pkg.all;
entity cycle_subrange_u is
  generic (L : positive);
  port (clk : in std_ulogic; a, b : in unsigned(W + 3 downto 0); q : out unsigned(W + 3 downto 0);
        eq, par : out std_ulogic);
end entity;
architecture a of cycle_subrange_u is
  subtype lo_r is natural range width_of(L) - 1 downto 0;
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= resize(a(lo_r) + b(lo_r), q'length) xor shift_left(a, 1);
    end if;
  end process;
  cmp : process (all)
    variable p : std_ulogic;
  begin
    eq <= '1' when a(lo_r) = b(lo_r) else '0';
    p := '0';
    for i in lo_r loop
      p := p xor a(i) xor b(i);
    end loop;
    par <= p;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_subrange is
end entity;
architecture t of cycle_subrange is
  type lens_t is array (0 to 2) of positive;
  constant lens : lens_t := (1, W, W + 4);
  type vecs_t is array (0 to 2) of unsigned(W + 3 downto 0);
  signal clk : std_ulogic := '0';
  signal a, b : unsigned(W + 3 downto 0) := (others => '0');
  signal q : vecs_t;
  signal eq, par : std_ulogic_vector(0 to 2);
begin
  g : for k in 0 to 2 generate
    u : entity work.cycle_subrange_u generic map (L => lens(k))
      port map (clk => clk, a => a, b => b, q => q(k), eq => eq(k), par => par(k));
  end generate;
  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    for c in 1 to 24 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      a <= a(W + 2 downto 0) & r(0);
      if c mod 3 = 0 then
        b <= a;
      else
        b <= b rol 1;
      end if;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "c=" & integer'image(c) & " q0=" & to_hstring(q(0)) & " q1=" & to_hstring(q(1))
        & " q2=" & to_hstring(q(2)) & " eq=" & to_string(eq) & " par=" & to_string(par);
    end loop;
    wait;
  end process;
end architecture;
