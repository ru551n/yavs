-- difftest: top=port_conv_inout_formal_only std=2008 ref=hand
-- LRM 6.5.7.1: an inout port with a conversion on the formal side only (id(q) => io). Copy-out
-- converts (the driven 3 reaches io as 4), copy-in passes the actual unchanged, so q reads 4.
-- ref=hand: the first reference simulator leaves q at its own driven 3; the third agrees with 4.
entity port_conv_inout_formal_only_sub is port (q : inout integer); end entity;
architecture a of port_conv_inout_formal_only_sub is
begin
  process begin
    wait for 1 ns;
    q <= 3;
    wait for 1 ns;
    report "sub q " & integer'image(q);
    wait;
  end process;
end architecture;
entity port_conv_inout_formal_only is end entity;
architecture t of port_conv_inout_formal_only is
  signal io : integer := 0;
  function plus1(v : integer) return integer is begin return v + 1; end function;
begin
  u : entity work.port_conv_inout_formal_only_sub port map (plus1(q) => io);
  process begin
    wait for 3 ns;
    report "io " & integer'image(io);
    assert io = 4 severity failure;
    wait;
  end process;
end architecture;
