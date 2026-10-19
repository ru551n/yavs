-- LRM 2008 clause 17 / PSL: a directive clocked with `@`, no default clock.
entity top is end entity;
architecture a of top is
  signal clk : bit;
  signal x : bit := '1';
begin
  clk <= '1' after 1 ns, '0' after 2 ns;
  assert always x @rising_edge(clk);
  process begin report "RES ran"; wait; end process;
end architecture;
