-- yavs-test: std=2019
-- expect: none
-- LRM 5.6.2, 8.3: method calls on a protected variable and through an access value
-- (an access to a protected type is 2019, 5.4.1; E1221 in 2008)
package p is
  procedure t;
end package;
package body p is
  procedure t is
    type pt is protected
      impure function get return integer;
      impure function at (i : integer) return integer;
      procedure set (v : integer);
    end protected;
    type pt is protected body
      variable x : integer := 0;
      impure function get return integer is begin return x; end function;
      impure function at (i : integer) return integer is begin return x + i; end function;
      procedure set (v : integer) is begin x := v; end procedure;
    end protected body;
    type pt_ptr is access pt;
    variable v : pt;
    variable a : pt_ptr;
    variable i : integer;
  begin
    v.set(3);
    i := v.get + v.at(2);
    a.set(i);
    i := a.get;
    i := a.at(i => 1);
  end procedure;
end package body;
