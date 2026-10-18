-- yavs-test: std=2008
-- expect: E1502 9:3 generic `n` has no actual and no default
-- LRM 6.5.7.2
entity sub is generic (n : integer); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
begin
  u : entity work.sub;
end architecture;
