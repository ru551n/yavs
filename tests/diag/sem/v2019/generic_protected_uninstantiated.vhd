-- yavs-test: std=2019
-- expect: E1505 15:35 `pt` is a generic protected type and must be instantiated before use
-- LRM 2019 5.6.2
package p is
  type pt is protected
    generic (type t);
    procedure set (x : t);
  end protected;
end package;
package body p is
  type pt is protected body
    procedure set (x : t) is begin end procedure;
  end protected body;
end package body;
package q is shared variable sv : work.p.pt; end package;
