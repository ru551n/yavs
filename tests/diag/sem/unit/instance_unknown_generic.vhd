-- yavs-test: std=2008
-- expect: E1100 9:36 entity `sub` has no generic `m`
-- LRM 6.5.7
entity sub is generic (n : integer := 1); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
begin
  u : entity work.sub generic map (m => 2);
end architecture;
