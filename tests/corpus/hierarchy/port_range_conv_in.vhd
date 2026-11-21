-- difftest: top=port_range_conv_in std=2008 exit=1 ref=hand
-- ref=hand: sim1 lets the port read the converted -12, sim2 rejects the association at
-- elaboration (discrepancy `port-range`). The result of the conversion of an in port's actual is the
-- port's effective value and belongs to the port's subtype (LRM 6.5.7.1, 14.7.3.3, #1183).
entity port_range_conv_in_dst is
  port (i : in natural);
end entity;
architecture a of port_range_conv_in_dst is
begin
  process (i)
  begin
    report "i=" & integer'image(i);
  end process;
end architecture;

entity port_range_conv_in is end entity;
architecture a of port_range_conv_in is
  signal s : integer := 2;
  function minus2 (x : integer) return integer is begin return x - 2; end function;
begin
  u : entity work.port_range_conv_in_dst port map (i => minus2(s));
  process
  begin
    s <= 7;
    wait for 1 ns;
    s <= -10;
    wait for 1 ns;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
