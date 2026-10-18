-- yavs-test: std=2019
-- expect: E1703 13:28 component generic `t` and the generic of entity `sub` have different types
-- LRM 7.3.3 (default generic map of a binding indication), 6.5.6.2
entity sub is
  generic (t : integer := 0);
end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component sub is
    generic (type t);
  end component;
  for all : sub use entity work.sub(r);
begin
end architecture;
