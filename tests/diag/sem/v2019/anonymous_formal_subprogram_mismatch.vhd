-- yavs-test: std=2019
-- expect: E1300 14:50 profile of `two` does not match generic subprogram `cmp`
-- expect: E1300 15:50 profile of `eight` does not match generic subprogram `cmp`
-- expect: E1300 16:50 profile of `tm` does not match generic subprogram `cmp`
-- LRM 2019 6.5.4: both parameters of `cmp` have the same type, of the class of the anonymous type
package p is
  generic (function cmp (a : type is range <>; b : a'subtype) return boolean);
end package;
entity top is end;
architecture a of top is
  function two (a : integer; b : boolean) return boolean is begin return a = 1; end function;
  function eight (b : boolean) return boolean is begin return b; end function;
  function tm (a, b : bit) return boolean is begin return a = b; end function;
  package bad1 is new work.p generic map (cmp => two);
  package bad2 is new work.p generic map (cmp => eight);
  package bad3 is new work.p generic map (cmp => tm);
begin
end architecture;
