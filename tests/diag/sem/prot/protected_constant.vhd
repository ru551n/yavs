-- yavs-test: std=2008
-- expect: E1905 16:12 a constant cannot have protected type `pt`
-- LRM 5.6.2
package p is
  type pt is protected
    procedure set (x : integer);
  end protected;
end package;
package body p is
  type pt is protected body
    procedure set (x : integer) is begin null; end procedure;
  end protected body;
end package body;
use work.p.all;
package q is
  constant k : pt;
end package;
