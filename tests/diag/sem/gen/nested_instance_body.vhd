-- yavs-test: std=2008
-- expect: E1004 11:16 package `i` is not declared in this region
-- LRM 4.8: a package instance has no body of its own
entity e is end entity;
architecture a of e is
  package g is
    generic (x : integer);
    constant c : integer := x;
  end package;
  package i is new g generic map (x => 1);
  package body i is
  end package body;
begin
end architecture;
