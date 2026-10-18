-- yavs-test: std=2019
-- expect: none
-- LRM 7.3.3: a component's generic types, subprograms and packages bind by name.
package gp is
  generic (n : integer);
  constant c : integer := n;
end package;
entity sub is
  generic (type t; v : t; function f(x : t) return t;
           package p is new work.gp generic map (<>));
  port (q : out t);
end entity;
architecture r of sub is begin q <= f(v); end architecture;
entity e is end entity;
architecture a of e is
  component sub is
    generic (type t; v : t; function f(x : t) return t;
             package p is new work.gp generic map (<>));
    port (q : out t);
  end component;
  for all : sub use entity work.sub(r);
begin
end architecture;
