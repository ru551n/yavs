-- yavs-test: std=2008
-- expect: E1313 14:61 inout port converted the way out only
-- help: convert both ways
-- LRM 6.5.7.1
-- An `inout` port with a conversion on the formal side only.
entity port_conv_inout_one_way_out_sub is port (q : inout integer); end entity;
architecture a of port_conv_inout_one_way_out_sub is begin q <= 3; end architecture;
entity e is end entity;
architecture a of e is
  signal io : integer := 0;
  function id(v : integer) return integer is begin return v + 1; end function;
begin
  -- The actual's value would come in to `q` without a conversion.
  u : entity work.port_conv_inout_one_way_out_sub port map (id(q) => io);
end architecture;
