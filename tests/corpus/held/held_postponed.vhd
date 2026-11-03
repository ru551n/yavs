-- difftest: top=held_postponed std=2008 exit=1 ignore=order,fatalmsg
-- ignore=order: reports of different processes and instances in one cycle.
-- A held zero-delay drive in a postponed process: the error stays.
library ieee; use ieee.std_logic_1164.all;
entity held_postponed_u is generic (g : natural); end;
architecture a of held_postponed_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 100 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  postponed process (clk) begin
    if clk = '1' then
      if n < 0 or g > 1000 then report "n"; end if;
      report "pp " & integer'image(n) & std_logic'image(s);
      if n >= 5 then s <= s; end if;
    end if;
  end process;
end;
entity held_postponed is end;
architecture a of held_postponed is
begin
  u0 : entity work.held_postponed_u generic map (0);
  u1 : entity work.held_postponed_u generic map (1);
end;
