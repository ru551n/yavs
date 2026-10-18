-- yavs-test: std=2019
-- expect: E1703 21:28 component generic `p` and the generic of entity `sub` have different types
-- LRM 7.3.3 (default generic map of a binding indication), 6.5.7.2
package gp1 is
  generic (n : integer);
  constant c : integer := n;
end package;
package gp2 is
  generic (n : integer);
  constant c : integer := n;
end package;
entity sub is
  generic (package p is new work.gp1 generic map (<>));
end entity;
architecture r of sub is begin end architecture;
entity e is end entity;
architecture a of e is
  component sub is
    generic (package p is new work.gp2 generic map (<>));
  end component;
  for all : sub use entity work.sub(r);
begin
end architecture;
