entity limited is
  port (clk : in bit);
end entity;

architecture rtl of limited is
  signal n : integer := integer'high - 2;
begin
  process (clk)
  begin
    if clk'event and clk = '1' then
      n <= n + 1;  -- overflows at the third edge
    end if;
  end process;
end architecture;

entity tb_overflow is
end entity;

architecture tb of tb_overflow is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  dut : entity work.limited port map (clk => clk);
  process
  begin
    wait for 100 ns;
    std.env.finish;
  end process;
end architecture;
