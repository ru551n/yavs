-- yavs-test: std=2008
-- expect: E1224 6:12 constant `c` needs an initial value
-- LRM 6.4.2.2
entity e is end entity;
architecture a of e is
  constant c : bit;
begin
end architecture;
