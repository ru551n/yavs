-- yavs-test: std=2019
-- expect: none
-- LRM 2019 5.6.2: protected type instantiation with a generic map
package p is
  type pt is protected
    generic (type t; constant n : natural);
    procedure set (x : t);
    impure function size return natural;
  end protected;
end package;
package body p is
  type pt is protected body
    procedure set (x : t) is begin end procedure;
    impure function size return natural is begin return n; end function;
  end protected body;
end package body;
package q is
  type ipt is new work.p.pt generic map (t => integer, n => 3);
  type bpt is new work.p.pt generic map (bit, 1);
  shared variable sv : ipt;
end package;
