-- yavs-test: std=2019
-- expect: E1903 20:3 element `a` of port `x` has mode `in` and cannot be driven
-- LRM 2019 6.5.2
package p is
  type rec is record
    a : bit;
    b : bit;
  end record;
  view v of rec is
    a : in;
    b : out;
  end view;
end package;
use work.p.all;
entity e is
  port (x : view v of rec);
end entity;
architecture a of e is
begin
  x.a <= '1';
end architecture;
