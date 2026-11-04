-- difftest: top=held_deltas std=2008 stop=200ns ignore=order
-- Held drives across delta and time-step boundaries, inside one activation: the numbers of the
-- deltas are what a process and a postponed process see.
-- ignore=order: reports of different processes in one cycle.
library ieee; use ieee.std_logic_1164.all;
entity held_deltas_u is generic (g : natural); end;
architecture a of held_deltas_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal x, y, z : std_logic := '0';
  signal v : std_logic_vector(7 downto 0) := x"00";
  signal w : std_logic_vector(69 downto 0) := (others => '0');
  signal i : integer := 0;
begin
  clk <= not clk after 5 ns when now < 180 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  -- Suspends inside the window: held, a zero delay wait, changed, a zero delay wait, held again.
  process begin
    wait until rising_edge(clk);
    if n < 0 or g > 1000 then report "n"; end if;
    x <= x;
    v <= v;
    i <= i;
    wait for 0 ns;
    report "d1 x=" & std_logic'image(x) & " i=" & integer'image(i);
    if n mod 3 = 1 then
      x <= not x;
      v(3 downto 2) <= not v(3 downto 2);
      i <= i + 1;
    end if;
    wait for 0 ns;
    report "d2 x=" & std_logic'image(x) & " v=" & to_string(v) & " i=" & integer'image(i);
    x <= x;
    v(3 downto 2) <= v(3 downto 2);
    i <= i;
    wait for 0 ns;
    report "d3 x=" & std_logic'image(x) & " v=" & to_string(v) & " i=" & integer'image(i);
    wait for 1 ns;
    x <= x;
    i <= i;
    wait for 1 ps;
    report "t2 x=" & std_logic'image(x);
  end process;
  -- A signal read at once after it is driven: the value of before.
  process (clk) begin
    if rising_edge(clk) then
      if n < 0 or g > 1000 then report "n"; end if;
      y <= y;
      y <= not y;
      assert y = y;
      y <= y;
      w(69 downto 60) <= w(69 downto 60);
      w(63) <= not w(63);
      w(62 downto 61) <= w(62 downto 61);
      w(64) <= w(64);
      if n mod 5 = 0 then w(66 downto 58) <= not w(66 downto 58); end if;
    end if;
  end process;
  process (y) begin report "y=" & std_logic'image(y); end process;
  process (w) begin report "w=" & to_string(w(69 downto 56)); end process;
  postponed process (clk, x, v, i) begin report "pp " & std_logic'image(x) & to_string(v) & integer'image(i); end process;
end;
entity held_deltas is end;
architecture a of held_deltas is
begin
  u0 : entity work.held_deltas_u generic map (0);
  u1 : entity work.held_deltas_u generic map (1);
end;
