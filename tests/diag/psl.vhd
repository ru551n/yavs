-- yavs: --std 2019
architecture rtl of e is
begin
  assume always req -> next ack;
  ok <= '1';
end architecture;

vunit check (e) {
  assert always a;
}
