-- difftest: top=held_reject std=2008 exit=1 ignore=order,fatalmsg
-- ignore=order: reports of different processes and instances in one cycle.
-- A rejection limit above a zero delay is an error, also when the value is the one held.
library ieee; use ieee.std_logic_1164.all;
entity held_reject_u is generic (g : natural); end;
architecture a of held_reject_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := x"0";
  signal i : integer := 0;
begin
  clk <= not clk after 5 ns when now < 100 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk)
    variable r : time := 0 ns;
  begin
    if rising_edge(clk) then
      if n < 0 or g > 1000 then report "n"; end if;
      s <= reject r inertial s;
      v <= reject r inertial v;
      i <= reject r inertial i;
      report "ok at " & integer'image(n);
      if n = 6 then r := 1 ns; end if;
    end if;
  end process;
end;
entity held_reject is end;
architecture a of held_reject is
begin
  u0 : entity work.held_reject_u generic map (0);
  u1 : entity work.held_reject_u generic map (1);
end;
