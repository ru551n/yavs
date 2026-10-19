-- yavs-test: std=2019
-- expect: E1501 15:19 `pt` is not a generic protected type
-- LRM 2019 5.6.2: only a generic protected type can be instantiated
package p is
  type pt is protected
    procedure set (x : integer);
  end protected;
end package;
package body p is
  type pt is protected body
    procedure set (x : integer) is begin end procedure;
  end protected body;
end package body;
package q is
  type ipt is new work.p.pt generic map (t => integer);
end package;
