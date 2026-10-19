-- yavs-test: std=2008
-- expect: E1313 14:65 inout port converted the way in only
-- help: convert both ways
-- LRM 6.5.7.1
-- An `inout` port with a conversion on the actual side only.
entity port_conv_inout_one_way_in_sub is port (q : inout integer); end entity;
architecture a of port_conv_inout_one_way_in_sub is begin q <= 3; end architecture;
entity e is end entity;
architecture a of e is
  signal io : integer := 0;
  function id(v : integer) return integer is begin return v + 1; end function;
begin
  -- The formal's value would go back to `io` without a conversion.
  u : entity work.port_conv_inout_one_way_in_sub port map (q => id(io));
end architecture;
