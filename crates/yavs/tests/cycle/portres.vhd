-- An `out` port of a resolved subtype (`std_logic_vector`) on an unresolved actual
-- (`std_ulogic_vector`), against the normal mode (`W` replaced by a width): with one driver of
-- each element (a register, a concurrent copy) its writers join; with two drivers of an element
-- (resolved to 'X' where they differ) they stay in the event kernel.
library ieee;
use ieee.std_logic_1164.all;
entity cycle_portres_u is
  port (clk : in std_ulogic; d : in std_ulogic_vector(W - 1 downto 0);
        q : out std_logic_vector(W - 1 downto 0) := (others => '0'); c : out std_logic_vector(W - 1 downto 0);
        two : out std_logic_vector(W - 1 downto 0));
end entity;
architecture a of cycle_portres_u is
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
    end if;
  end process;
  c <= not d;
  two <= d;
  two <= (others => '1');
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity cycle_portres is
end entity;
architecture t of cycle_portres is
  signal clk : std_ulogic := '0';
  signal d, q, c, two, qq : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  u : entity work.cycle_portres_u port map (clk => clk, d => d, q => q, c => c, two => two);
  -- A member reading the actual.
  follow : process (clk)
  begin
    if rising_edge(clk) then
      qq <= q xor c;
    end if;
  end process;
  main : process
    variable r : std_ulogic_vector(15 downto 0) := x"ACE1";
  begin
    for k in 1 to 12 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      d <= d(W - 2 downto 0) & r(0);
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " q=" & to_hstring(q) & " c=" & to_hstring(c)
        & " qq=" & to_hstring(qq) & " two=" & to_string(two);
    end loop;
    wait;
  end process;
end architecture;
