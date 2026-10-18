-- yavs-test: std=2008
-- expect: E1300 10:10 expected `integer`, found `boolean`
-- LRM 6.5.7.1
entity e is end entity;
architecture a of e is
  function f (x : integer) return integer is begin return x; end function;
  signal s : integer;
  signal b : boolean;
begin
  s <= f(b);
end architecture;
