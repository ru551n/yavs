-- yavs-test: std=2008
-- expect: E1410 8:24 integer type bound must be locally static
-- LRM 5.2.3.1
entity e is
  generic (n : integer := 4);
end entity;
architecture a of e is
  type t is range 0 to n;
begin
end architecture;
