-- difftest: top=logic_reads_edges std=2008
-- Reads of single elements of std_logic signals and rising_edge / falling_edge, in processes
-- resumed by several signals and through ports that are slices of their actuals (the
-- element offset of an instance signal), with all nine values on the signals.
library ieee;
use ieee.std_logic_1164.all;

entity unit is
  port (clk : in std_logic; en : in std_logic; d : in std_logic_vector(3 downto 0);
        q : out std_logic_vector(3 downto 0); o : out std_logic);
end;

architecture a of unit is
begin
  process (clk, en)
  begin
    if rising_edge(clk) then
      q <= not d;
    end if;
    if falling_edge(clk) and en = '1' then
      o <= d(0) xor d(3);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;

entity logic_reads_edges is end;

architecture t of logic_reads_edges is
  signal clk, en : std_logic := '0';
  signal big : std_logic_vector(0 to 99) := (others => '0');
  signal q1, q2 : std_logic_vector(3 downto 0);
  signal o1, o2 : std_logic;
  signal s : std_ulogic := 'U';
  constant all9 : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
begin
  clk <= not clk after 5 ns when now < 120 ns;

  u1: entity work.unit port map (clk, en, big(10 to 13), q1, o1);
  u2: entity work.unit port map (clk, en, big(96 to 99), q2, o2);

  stim: process
  begin
    for i in 0 to 14 loop
      wait for 7 ns;
      big(10 to 13) <= all9((i + 0) mod 9) & all9((i + 3) mod 9) & all9((i + 5) mod 9) & all9((i + 8) mod 9);
      big(96 to 99) <= all9((i + 1) mod 9) & all9((i + 2) mod 9) & all9((i + 4) mod 9) & all9((i + 6) mod 9);
      s <= all9(i mod 9);
      if i = 5 then en <= '1'; end if;
    end loop;
    wait;
  end process;

  -- Element reads in a process of several triggers.
  mon: process (clk, s, en)
    variable n : natural := 0;
  begin
    n := n + 1;
    report integer'image(n) & " clk=" & std_ulogic'image(clk) & " s=" & std_ulogic'image(s)
      & " e=" & boolean'image(rising_edge(clk)) & boolean'image(falling_edge(clk))
      & " b10=" & std_ulogic'image(big(10)) & " b13=" & std_ulogic'image(big(13))
      & " b96=" & std_ulogic'image(big(96)) & " b99=" & std_ulogic'image(big(99))
      & " q1=" & to_string(q1) & " q2=" & to_string(q2)
      & " o=" & std_ulogic'image(o1) & std_ulogic'image(o2);
  end process;
end;
