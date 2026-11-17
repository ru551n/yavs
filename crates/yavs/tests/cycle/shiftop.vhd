-- The shift operators `sll`, `srl`, `rol`, `ror` of `numeric_std` (unsigned, signed: `srl`
-- logical) and `std_logic_1164` (`1 to n` results), of any width (`W` replaced by a width), by
-- counts chosen at run time from 0 past twice the width and by negative counts known here; bits of
-- results by index. A count that may be negative at run time keeps its process out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shiftop_u is
  port (clk : in std_ulogic; x : in std_ulogic_vector(W - 1 downto 0); n : in natural;
        m : in integer; c, q : out std_ulogic_vector(10 * W + 1 downto 0);
        y : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_shiftop_u is
begin
  comb : process (all)
    variable u : unsigned(W - 1 downto 0);
    variable s : signed(W - 1 downto 0);
  begin
    u := unsigned(x);
    s := signed(x);
    c <= std_ulogic_vector(u sll n) & std_ulogic_vector(u srl n) & std_ulogic_vector(u rol n)
      & std_ulogic_vector(u ror n) & std_ulogic_vector(s srl n) & std_ulogic_vector(s sll n)
      & (x sll n) & (x srl n) & (x rol n) & (x ror n)
      & "sll"(x, n)(1) & "srl"(u, 1)(0);
  end process;
  reg : process (clk)
    constant k : integer := -3;
  begin
    if rising_edge(clk) then
      q <= std_ulogic_vector(unsigned(x) sll k) & std_ulogic_vector(signed(x) srl -2)
        & std_ulogic_vector(signed(x) sll -1) & std_ulogic_vector(unsigned(x) rol -5)
        & std_ulogic_vector(unsigned(x) ror (2 * W + 1)) & (x sll -2) & (x srl k)
        & (x rol -1) & (x ror -7) & (x srl 2 * W) & "rol"(x, -1)(1) & "sll"(signed(x), -1)(0);
    end if;
  end process;
  neg : process (all)
  begin
    y <= std_ulogic_vector(unsigned(x) sll m);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_shiftop is
end entity;
architecture t of cycle_shiftop is
  signal clk : std_ulogic := '0';
  signal x : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal n : natural := 0;
  signal m : integer := 0;
  signal c, q : std_ulogic_vector(10 * W + 1 downto 0);
  signal y : std_ulogic_vector(W - 1 downto 0);
begin
  u : entity work.cycle_shiftop_u port map (clk, x, n, m, c, q, y);
  main : process
    variable s : unsigned(31 downto 0) := x"ACE1F00D";
  begin
    for k in 0 to 2 * W + 3 loop
      for b in x'range loop
        s := s(30 downto 0) & (s(31) xor s(21) xor s(1) xor s(0));
        x(b) <= s(0);
      end loop;
      n <= k;
      m <= k - W;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " " & to_hstring(c) & " " & to_hstring(q) & " "
        & to_hstring(y);
    end loop;
    wait;
  end process;
end architecture;
