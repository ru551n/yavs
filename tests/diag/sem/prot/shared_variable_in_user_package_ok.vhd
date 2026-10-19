-- yavs-test: std=2008
-- expect: none
-- LRM 14.4.2
package p is
  type pt is protected
    impure function get return natural;
  end protected;
end package;
package body p is
  type pt is protected body
    impure function get return natural is begin return 1; end function;
  end protected body;
end package body;
use work.p.all;
package q is
  shared variable sv : pt;
end package;
