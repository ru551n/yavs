-- A counter driven only through VHPI (clock and reset come from the test plugin put.c),
-- and a signal driven by the design that the plugin forces, releases and deposits (#271).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter is
  port (clk, rst : in std_logic; q : out unsigned(3 downto 0));
end;

architecture a of counter is
  signal n : unsigned(3 downto 0) := (others => '0');
begin
  process (clk) begin
    if rising_edge(clk) then
      if rst = '1' then
        n <= (others => '0');
      else
        n <= n + 1;
      end if;
    end if;
  end process;
  q <= n;
end;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity top is
  generic (g : integer := 3);
end;

architecture a of top is
  type pair is record
    a : integer;
    b : std_logic;
  end record;
  constant c : integer := 5;
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal q : unsigned(3 downto 0);
  signal d : integer := 0;
  signal p : pair := (0, '0');
  signal str : string(1 to 8) := "abcdefgh";
  signal e : std_ulogic_vector(7 downto 0) := (others => '0');
  signal fa, fb, fc, fd : std_ulogic := '0';
begin
  u : entity work.counter port map (clk, rst, q);
  d <= 1 after 10 ns, 2 after 30 ns, 3 after 50 ns, 4 after 70 ns;
  process (d) begin
    report "d=" & integer'image(d);
  end process;
  process (rst) begin
    report "rst=" & std_logic'image(rst);
  end process;
  pv : process
    variable v : integer := 0;
  begin
    wait;
  end process;
  e <= "01011010" after 10 ns, "00000000" after 20 ns;
  fa <= '1' after 10 ns, '0' after 30 ns, '1' after 50 ns;
  fb <= '1' after 10 ns, '0' after 30 ns, '1' after 50 ns;
  fc <= '1' after 10 ns, '0' after 30 ns, '1' after 50 ns;
  fd <= '1' after 10 ns, '0' after 30 ns, '1' after 50 ns;
end;
