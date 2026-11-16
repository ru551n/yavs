library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity compare is
  port (
    clk   : in  std_logic;
    a, b  : in  unsigned(3 downto 0);
    eq    : out std_logic;
    ticks : out unsigned(3 downto 0)
  );
end entity;

architecture rtl of compare is
  signal t : unsigned(3 downto 0);  -- no initial value: 'U' in the normal mode
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if a = b then
        eq <= '1';
      else
        eq <= '0';
      end if;
      t <= t + 1;
    end if;
  end process;
  ticks <= t;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_metavalue is
end entity;

architecture tb of tb_metavalue is
  signal clk   : std_logic := '0';
  signal x, y  : unsigned(3 downto 0) := (others => '0');
  signal eq    : std_logic;
  signal ticks : unsigned(3 downto 0);
begin
  clk <= not clk after 5 ns;

  dut : entity work.compare port map (clk => clk, a => x, b => y, eq => eq, ticks => ticks);

  main : process
  begin
    report "at 0 ns:  ticks=" & to_string(ticks);
    wait for 1 ns;
    report "at 1 ns:  ticks=" & to_string(ticks);
    wait for 11 ns;
    x <= (others => 'X');  -- a metavalue reaches the design
    wait for 5 ns;
    report "at 17 ns: eq=" & to_string(eq) & " with 'X' on a";
    std.env.finish;
  end process;
end architecture;
