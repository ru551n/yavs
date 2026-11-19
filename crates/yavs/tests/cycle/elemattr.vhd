-- Attributes of an element of a memory chosen at run time (`mem(row)'low`), whose index range is
-- its element type's, as slice bounds in an unrolled loop and in arithmetic, against the normal
-- mode (`W` replaced by a width): a weight memory model whose memory is a port of an
-- unconstrained array of unconstrained vectors, constrained by its actual.
library ieee;
use ieee.std_logic_1164.all;
package cycle_elemattr_pkg is
  type vec_t is array (natural range <>) of std_ulogic_vector;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.cycle_elemattr_pkg.all;
entity cycle_elemattr_u is
  generic (LANES : positive);
  port (clk : in std_ulogic; row : in natural range 0 to 7; mem : in vec_t;
        rd : out std_ulogic_vector(8 * LANES - 1 downto 0); n : out natural);
end entity;
architecture a of cycle_elemattr_u is
begin
  rdp : process (clk)
    variable w : std_ulogic_vector(8 * LANES - 1 downto 0);
    variable r : natural;
  begin
    if rising_edge(clk) then
      r := row;
      if r <= mem'high then
        for l in 0 to LANES - 1 loop
          w(8 * l + 7 downto 8 * l) := mem(r)(mem(r)'low + 8 * l + 7 downto mem(r)'low + 8 * l);
        end loop;
      end if;
      rd <= w;
      n <= mem(r)'length + mem(r)'high - mem(r)'right;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.cycle_elemattr_pkg.all;
entity cycle_elemattr is
end entity;
architecture t of cycle_elemattr is
  constant LANES : positive := 4;
  signal mem : vec_t(0 to 7)(8 * LANES + W - 1 downto W) := (others => (others => '0'));
  signal clk : std_ulogic := '0';
  signal row : natural range 0 to 7 := 0;
  signal rd : std_ulogic_vector(8 * LANES - 1 downto 0);
  signal n : natural;
begin
  u : entity work.cycle_elemattr_u generic map (LANES => LANES)
    port map (clk => clk, row => row, mem => mem, rd => rd, n => n);
  main : process
    variable s : unsigned(31 downto 0) := x"ACE1F00D";
  begin
    for k in 0 to 7 loop
      s := s(30 downto 0) & (s(31) xor s(21) xor s(1) xor s(0));
      mem(k) <= std_ulogic_vector(s);
    end loop;
    for k in 1 to 10 loop
      row <= (k * 3) mod 8;
      wait for 1 ns;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 4 ns;
      report "k=" & integer'image(k) & " rd=" & to_hstring(rd) & " n=" & integer'image(n);
    end loop;
    wait;
  end process;
end architecture;
