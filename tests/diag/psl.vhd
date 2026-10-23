-- yavs: --std 2019
architecture rtl of e is
  default clock is rising_edge(clk);
begin
  assume always req -> next ack;
  assert eventually! req;
  ok <= '1';
end architecture;

vunit check (e) {
  assert always a;
}
