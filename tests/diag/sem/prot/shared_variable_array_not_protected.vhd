-- yavs-test: std=2008
-- expect: E1904 6:19 shared variable `sv` must be of a protected type
-- LRM 6.4.2.4
entity e is end entity;
architecture a of e is
  shared variable sv : bit_vector(0 to 3);
begin
end architecture;
