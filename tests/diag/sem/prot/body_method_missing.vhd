-- yavs-test: std=2008
-- expect: E1230 7:21 method `get` of protected type `pt` has no body
-- LRM 5.6.3
package p is
  type pt is protected
    procedure set (x : integer);
    impure function get return integer;
  end protected;
end package;
package body p is
  type pt is protected body
    variable v : integer;
    procedure set (x : integer) is begin v := x; end procedure;
  end protected body;
end package body;
