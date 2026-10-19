-- yavs-test: std=2008
-- expect: E1230 9:19 shared variable of protected type `pt` declared before its body
-- LRM 14.4.2
package p is
  type pt is protected
    impure function get return natural;
  end protected;
  -- the body is in the package body, not elaborated yet
  shared variable sv : pt;
end package;
package body p is
  type pt is protected body
    impure function get return natural is begin return 1; end function;
  end protected body;
end package body;
