-- yavs-test: std=2008
-- expect: E1100 9:51 entity `sub` has no generic `m`
-- LRM 7.3.2
entity sub is generic (n : integer := 1); end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component c is generic (m : integer := 1); end component;
  for all : c use entity work.sub(r) generic map (m => m);
begin
end architecture;
