-- yavs-test: std=2019
-- expect: E1300 16:47 expected `INTEGER`, found a string literal
-- LRM 2019 5.6.2, 6.5.7.2: the actual of a generic constant has the formal's type
package p is
  type pt is protected
    generic (constant n : natural);
    impure function size return natural;
  end protected;
end package;
package body p is
  type pt is protected body
    impure function size return natural is begin return n; end function;
  end protected body;
end package body;
package q is
  type ipt is new work.p.pt generic map (n => "abc");
end package;
