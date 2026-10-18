-- yavs-test: std=2008
-- expect: E1311 9:3 port `p` of mode `in` has no actual
-- An open unconstrained in port needs a default (#218); with one it takes the default's bounds.
entity sub is port (p : in bit_vector); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
begin
  u : entity work.sub;
end architecture;
