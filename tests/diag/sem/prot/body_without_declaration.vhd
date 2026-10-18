-- yavs-test: std=2008
-- expect: E1100 7:8 protected type `pt` is not declared
-- LRM 5.6.3
package p is
end package;
package body p is
  type pt is protected body
  end protected body;
end package body;
