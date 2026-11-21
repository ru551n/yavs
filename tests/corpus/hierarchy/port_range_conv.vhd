-- difftest: top=port_range_conv std=2008 exit=1 ref=hand
-- ref=hand: sim1 drives the converted 10 into the actual with no check, sim2 rejects the
-- association at elaboration (discrepancy `port-range`). The result of the conversion of an out port's driving
-- value is the driving value of the actual and belongs to its subtype (LRM 6.5.7.1, 14.7.3,
-- #1183): 4 goes through as 8, 5 converts to 10, outside of `integer range 0 to 9`.
entity port_range_conv_src is
  port (o : out integer := 0);
end entity;
architecture a of port_range_conv_src is
begin
  process
  begin
    o <= 4;
    wait for 1 ns;
    o <= 5;
    wait;
  end process;
end architecture;

entity port_range_conv is end entity;
architecture a of port_range_conv is
  signal s : integer range 0 to 9 := 0;
  function twice (x : integer) return integer is begin return 2 * x; end function;
begin
  u : entity work.port_range_conv_src port map (twice(o) => s);
  process
  begin
    wait for 0 ns;
    report "s=" & integer'image(s);
    wait for 2 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
