-- yavs-test: std=2008
-- expect: E1301 14:75 `inout` port `q` of type `INTEGER` is converted the way out only
-- help: convert the actual too
-- LRM 6.5.7.1
-- An `inout` port with a formal conversion only passes the actual in unchanged, so the actual
-- must have the formal's type.
entity port_conv_inout_formal_only_types_sub is port (q : inout integer); end entity;
architecture a of port_conv_inout_formal_only_types_sub is begin q <= 3; end architecture;
entity e is end entity;
architecture a of e is
  signal io : real := 0.0;
  function f(v : integer) return real is begin return real(v); end function;
begin
  u : entity work.port_conv_inout_formal_only_types_sub port map (f(q) => io);
end architecture;
