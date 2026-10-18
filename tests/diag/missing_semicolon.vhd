entity counter is
  port (clk : in bit);
end entity;

architecture rtl of counter is
  signal n : integer := 0
begin
  process (clk) begin
    n <= n + 1
  end process;
end architecture;
