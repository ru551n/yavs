-- LRM 2008 clause 17 / PSL: a PSL assertion in an architecture.
entity top is end entity;
architecture a of top is
  signal clk : bit;
  signal x : bit := '1';
  default clock is clk'event and clk = '1';
begin
  assert always x;
  process begin report "RES ran"; wait; end process;
end architecture;
