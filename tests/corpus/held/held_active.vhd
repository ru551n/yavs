-- difftest: top=held_active std=2008 stop=120ns ignore=order
-- ignore=order: reports of different processes and instances in one cycle.
-- Held drives keep 'active, 'last_active and 'event in the delta they make, read directly.
library ieee; use ieee.std_logic_1164.all;
entity held_active_u is generic (g : natural); end;
architecture a of held_active_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s : std_logic := '0';
  signal v : std_logic_vector(7 downto 0) := x"00";
  signal i : integer := 0;
begin
  clk <= not clk after 5 ns when now < 100 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk) begin
    if rising_edge(clk) then
      if n >= 0 or g > 1000 then
        s <= s;
        v <= v;
        v(3) <= v(3);
        i <= i;
      end if;
      if n mod 4 = 2 then s <= not s; i <= i + 1; v(2) <= not v(2); end if;
    end if;
  end process;
  process begin
    wait until clk = '1';
    wait for 0 ns;
    wait for 0 ns;
    report "active s " & boolean'image(s'active) & " v " & boolean'image(v'active) & " v3 " & boolean'image(v(3)'active)
      & " v2 " & boolean'image(v(2)'active) & " i " & boolean'image(i'active)
      & " la s " & time'image(s'last_active) & " v3 " & time'image(v(3)'last_active) & " v2 " & time'image(v(2)'last_active) & " i " & time'image(i'last_active)
      & " ev " & boolean'image(s'event) & " le " & time'image(s'last_event);
    wait for 1 ns;
    report "later s " & boolean'image(s'active) & " la " & time'image(s'last_active) & " v " & time'image(v'last_active);
  end process;
end;
entity held_active is end;
architecture a of held_active is
begin
  u0 : entity work.held_active_u generic map (0);
  u1 : entity work.held_active_u generic map (1);
end;
