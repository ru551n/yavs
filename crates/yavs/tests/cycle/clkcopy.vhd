-- Clocks copied by assignment are their source in cycle mode (A32): an aggregate of the clock,
-- a port connection and a plain copy; an inverted and a gated clock are no copies, nor is a copy
-- whose start value gives it an edge at time 0 that its source does not have.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_clkcopy_sub is
  port (clk : in std_logic; d : in unsigned(7 downto 0); q : out unsigned(7 downto 0));
end entity;
architecture rtl of cycle_clkcopy_sub is
begin
  reg : process (clk) begin
    if rising_edge(clk) then
      q <= d + 1;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

-- Copies made inside an entity from its clock port, chosen by a generic.
entity cycle_clkcopy_dist is
  generic (G : boolean := true);
  port (clk : in std_logic; d : in unsigned(7 downto 0); q : out unsigned(7 downto 0));
end entity;
architecture rtl of cycle_clkcopy_dist is
  signal cl : std_logic_vector(0 to 1);
  signal r, s : unsigned(7 downto 0) := (others => '0');
begin
  cl <= (others => clk) when G else (0 => clk, 1 => '0');
  reg0 : process (cl(0)) begin
    if rising_edge(cl(0)) then r <= d; end if;
  end process;
  s <= r + 2;
  reg1 : process (cl(1)) begin
    if rising_edge(cl(1)) then q <= s; end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity cycle_clkcopy is
end entity;
architecture tb of cycle_clkcopy is
  signal clk : std_logic := '0';
  signal hi : std_logic := '1';
  signal clks : std_logic_vector(3 downto 0);
  signal inv, gated, one, hcopy : std_logic;
  signal a, b, m, c, d, g, n, p, h, k, f, e, x, y, z, v, w : unsigned(7 downto 0) := (others => '0');
  signal div, dcopy, bsrc, b1, b2 : std_logic := '0';
  signal en : std_logic := '1';
begin
  clk <= not clk after 5 ns;
  hi <= not hi after 5 ns;
  clks <= (2 => en, others => clk);
  one <= clk;
  inv <= not clk;
  gated <= clk and en;
  hcopy <= hi;
  src : process (clk) begin
    if rising_edge(clk) then a <= a + 3; end if;
  end process;
  m <= a + 1;
  -- Logic of the source's registers sampled on copies: the normal mode samples it before it
  -- settles, as a register of the source would.
  reg_b : process (clks(1)) begin
    if rising_edge(clks(1)) then b <= m; end if;
  end process;
  u : entity work.cycle_clkcopy_sub port map (clk => clks(3), d => m, q => d);
  -- A register of the source read directly on a copy: the normal mode gives the new value.
  reg_c : process (one) begin
    if rising_edge(one) then c <= a; end if;
  end process;
  reg_n : process (inv) begin
    if rising_edge(inv) then n <= a; end if;
  end process;
  reg_g : process (gated) begin
    if rising_edge(gated) then g <= a; end if;
  end process;
  reg_p : process (clks(2)) begin
    if rising_edge(clks(2)) then p <= a; end if;
  end process;
  -- An edge test of `hi` keeps its history, so only its start value keeps `hcopy` its own.
  reg_k : process (hi) begin
    if rising_edge(hi) then k <= k + 1; end if;
  end process;
  -- 'U' to '1' at time 0 is an edge of this form on the copy, not on its source.
  reg_h : process (hcopy) begin
    if hcopy'event and hcopy = '1' then h <= h + 1; end if;
  end process;
  -- A falling edge from 'U' is none: `one` is still `clk` here.
  reg_f : process (one) begin
    if falling_edge(one) then f <= a; end if;
  end process;
  dist : entity work.cycle_clkcopy_dist port map (clk => clk, d => m, q => e);
  -- A copy of a clock a register of the region makes is no copy of it.
  reg_div : process (clk) begin
    if rising_edge(clk) then div <= not div; end if;
  end process;
  dcopy <= div;
  reg_x : process (dcopy) begin
    if rising_edge(dcopy) then x <= a; end if;
  end process;
  -- Two copies of a clock nothing else tests: one class, the logic between them joins.
  bsrc <= not bsrc after 4 ns;
  b1 <= bsrc;
  b2 <= bsrc;
  reg_y : process (b1) begin
    if rising_edge(b1) then y <= y + 5; end if;
  end process;
  v <= y + 1;
  reg_z : process (b2) begin
    if rising_edge(b2) then z <= v; end if;
  end process;
  w <= z;
  main : process begin
    for i in 0 to 12 loop
      wait for 7 ns;
      report "a=" & to_string(to_integer(a)) & " b=" & to_string(to_integer(b))
        & " c=" & to_string(to_integer(c)) & " d=" & to_string(to_integer(d))
        & " n=" & to_string(to_integer(n)) & " g=" & to_string(to_integer(g))
        & " p=" & to_string(to_integer(p)) & " h=" & to_string(to_integer(h))
        & " k=" & to_string(to_integer(k)) & " f=" & to_string(to_integer(f))
        & " e=" & to_string(to_integer(e)) & " x=" & to_string(to_integer(x))
        & " z=" & to_string(to_integer(w));
    end loop;
    std.env.finish;
  end process;
end architecture;
