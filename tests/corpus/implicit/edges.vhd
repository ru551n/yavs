-- difftest: top=edges std=2008
-- RISING_EDGE and FALLING_EDGE on BIT and BOOLEAN signals (LRM 2008 16.3) end to end (#163).
entity edges is end entity;

architecture test of edges is
  signal clk : bit := '0';
  signal en : boolean := false;
begin
  clk <= not clk after 5 ns when now < 30 ns;
  en <= true after 12 ns, false after 22 ns;

  process (clk)
  begin
    if rising_edge(clk) then
      report "rising";
    elsif falling_edge(clk) then
      report "falling";
    end if;
  end process;

  process (en)
  begin
    if rising_edge(en) then
      report "en rises";
    end if;
    if falling_edge(en) then
      report "en falls";
    end if;
  end process;
end architecture;
