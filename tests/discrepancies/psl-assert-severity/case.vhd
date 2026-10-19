-- LRM 2008 clause 17 / PSL: a `severity` clause on a PSL assertion.
entity top is end entity;
architecture a of top is
  signal clk : bit;
  signal x : bit := '1';
  default clock is clk'event and clk = '1';
begin
  clk <= '1' after 1 ns, '0' after 2 ns;
  assert always x report "x low" severity warning;
  process begin report "RES ran"; wait; end process;
end architecture;
