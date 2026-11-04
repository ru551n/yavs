-- difftest: top=held_ports std=2008 stop=150ns ignore=order
-- ignore=order: reports of different processes and instances in one cycle.
-- Held drives through aliases (reversed, sliced), out ports, open ports, conversions, and the
-- elements of arrays and records beside each other in a plane word.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity held_inner is
  generic (g : natural);
  port (clk : in std_logic; n : in natural; o : out std_logic_vector(3 downto 0); c : out unsigned(3 downto 0); u : out std_logic; i : in std_logic_vector(3 downto 0));
end;
architecture a of held_inner is
begin
  process (clk) begin
    if rising_edge(clk) then
      if n < 0 or g > 1000 then report "n"; end if;
      o <= o;
      o(2) <= o(2);
      c <= c;
      u <= u;
      if n mod 3 = 0 then o(1 downto 0) <= not i(1 downto 0); c <= c + 1; u <= '1'; end if;
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity held_ports_u is generic (g : natural); end;
architecture a of held_ports_u is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal big : std_logic_vector(0 to 15) := x"00f0";
  alias rev : std_logic_vector(15 downto 0) is big;
  alias hi : std_logic_vector(7 downto 0) is big(8 to 15);
  signal o1 : std_logic_vector(3 downto 0);
  signal c1 : std_logic_vector(3 downto 0);
  signal i1 : std_logic_vector(3 downto 0) := "0101";
  type rec is record a : std_logic; b : std_logic_vector(5 downto 0); c : integer; d : std_logic_vector(0 to 2); end record;
  signal rr : rec := ('0', "000111", 3, "010");
  type arr is array (1 to 3) of rec;
  signal ra : arr := (others => ('1', "101010", 1, "100"));
begin
  clk <= not clk after 5 ns when now < 130 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  u1 : entity work.held_inner generic map (g) port map (clk => clk, n => n, o => o1, std_logic_vector(c) => c1, u => open, i => i1);
  process (clk) begin
    if rising_edge(clk) then
      if n < 0 or g > 1000 then report "n"; end if;
      rev(3) <= rev(3);
      hi(5 downto 2) <= hi(5 downto 2);
      big(1) <= big(1);
      if n mod 4 = 1 then rev(3) <= not rev(3); end if;
      if n mod 7 = 1 then big(9) <= not big(9); end if;
      rr.b(4) <= rr.b(4);
      rr.d <= rr.d;
      rr.c <= rr.c;
      rr.a <= rr.a;
      if n mod 5 = 3 then rr.b(4) <= not rr.b(4); rr.d(1) <= not rr.d(1); end if;
      ra(2).b(3 downto 1) <= ra(2).b(3 downto 1);
      ra(3).d <= ra(3).d;
      ra(1).c <= ra(1).c;
      if n mod 6 = 2 then ra(2) <= ra(3); ra(2).c <= ra(2).c; end if;
    end if;
  end process;
  process (big, o1, c1, rr, ra) begin
    report to_string(big) & " " & to_string(o1) & " " & to_string(c1) & " " & std_logic'image(rr.a) & to_string(rr.b)
      & integer'image(rr.c) & to_string(rr.d) & " " & to_string(ra(2).b) & integer'image(ra(2).c) & to_string(ra(3).d) & integer'image(ra(1).c);
  end process;
end;
entity held_ports is end;
architecture a of held_ports is
begin
  u0 : entity work.held_ports_u generic map (0);
  u1 : entity work.held_ports_u generic map (1);
end;
